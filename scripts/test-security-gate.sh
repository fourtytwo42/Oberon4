#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SECURITY_GATE_SCRIPT="${ROOT_DIR}/scripts/oberon4-security-gate.sh"
WRAPPER_SCRIPT="${ROOT_DIR}/scripts/security-gate.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
RUN_ROOT="${TMP_DIR}/runs"
ORDER_FILE="${TMP_DIR}/order.log"
mkdir -p "${MOCK_BIN}" "${RUN_ROOT}"

cat > "${TMP_DIR}/gitleaks-baseline.json" <<'JSON'
[
  {
    "Fingerprint": "baseline-fingerprint"
  }
]
JSON

cat > "${TMP_DIR}/gitleaks.toml" <<'TOML'
title = "test"
TOML

cat > "${MOCK_BIN}/gitleaks" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf 'secret_scan\n' >>"${ORDER_FILE:?}"
report_path=""
baseline_path=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --report-path)
      report_path="$2"
      shift 2
      ;;
    --baseline-path)
      baseline_path="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
if [[ -n "${report_path}" ]]; then
  if [[ "${MOCK_SECRET_SCAN_NEW_FINDING:-0}" == "1" ]]; then
    printf '[{"Fingerprint":"new-fingerprint"}]\n' >"${report_path}"
  else
    cp "${baseline_path}" "${report_path}"
  fi
fi
if [[ "${MOCK_SECRET_SCAN_NEW_FINDING:-0}" == "1" ]]; then
  echo "gitleaks synthetic new finding" >&2
  exit 1
fi
echo "gitleaks ok"
SH
chmod +x "${MOCK_BIN}/gitleaks"

cat > "${MOCK_BIN}/semgrep" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf 'sast\n' >>"${ORDER_FILE:?}"
output_path=""
config_path=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      output_path="$2"
      shift 2
      ;;
    --config)
      config_path="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
if [[ "${config_path}" != "${EXPECTED_SEMGREP_CONFIG:?}" ]]; then
  echo "unexpected semgrep config: ${config_path}" >&2
  exit 1
fi
if [[ -n "${output_path}" ]]; then
  printf '{"results":[]}\n' >"${output_path}"
fi
if [[ "${MOCK_SAST_FAIL:-0}" == "1" ]]; then
  echo "semgrep synthetic failure" >&2
  exit 1
fi
echo "semgrep ok"
SH
chmod +x "${MOCK_BIN}/semgrep"

cat > "${MOCK_BIN}/cargo-deny" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf 'dependency_policy\n' >>"${ORDER_FILE:?}"
if [[ "${MOCK_DEPENDENCY_POLICY_FAIL:-0}" == "1" ]]; then
  echo "cargo-deny synthetic failure" >&2
  exit 1
fi
printf '{"cargo-deny":"ok"}\n'
SH
chmod +x "${MOCK_BIN}/cargo-deny"

cat > "${MOCK_BIN}/cargo-audit" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf 'dependency_audit\n' >>"${ORDER_FILE:?}"
if [[ "${MOCK_DEPENDENCY_AUDIT_FAIL:-0}" == "1" ]]; then
  echo "cargo-audit synthetic failure" >&2
  exit 1
fi
printf '{"vulnerabilities":{"found":false}}\n'
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

latest_run_dir() {
  find "${RUN_ROOT}" -mindepth 1 -maxdepth 1 -type d | sort | tail -n 1
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
  case "stages.secret_scan":
    value = summary.stages.find((s) => s.name === "secret_scan")?.status;
    break;
  case "stages.sast":
    value = summary.stages.find((s) => s.name === "sast")?.status;
    break;
  case "stages.dependency_audit":
    value = summary.stages.find((s) => s.name === "dependency_audit")?.status;
    break;
  case "stages.dependency_policy":
    value = summary.stages.find((s) => s.name === "dependency_policy")?.status;
    break;
  default:
    throw new Error(`unsupported expr: ${expr}`);
}
process.stdout.write(String(value));
NODE
}

run_gate() {
  local output_file="$1"
  shift
  : > "${ORDER_FILE}"
  env PATH="${MOCK_BIN}:$PATH" \
    ORDER_FILE="${ORDER_FILE}" \
    SECURITY_GATE_LOG_DIR="${RUN_ROOT}" \
    GITLEAKS_CONFIG_PATH="${TMP_DIR}/gitleaks.toml" \
    GITLEAKS_BASELINE_PATH="${TMP_DIR}/gitleaks-baseline.json" \
    SEMGREP_CONFIG_PATH="${TMP_DIR}/semgrep.yml" \
    CARGO_DENY_CONFIG_PATH="${TMP_DIR}/deny.toml" \
    EXPECTED_SEMGREP_CONFIG="${TMP_DIR}/semgrep.yml" \
    "$@" bash "${SECURITY_GATE_SCRIPT}" >"${output_file}" 2>&1
}

printf 'rules: []\n' > "${TMP_DIR}/semgrep.yml"
printf '[advisories]\nyanked = "deny"\n' > "${TMP_DIR}/deny.toml"

SUCCESS_OUTPUT="${TMP_DIR}/success.out"
run_gate "${SUCCESS_OUTPUT}"
SUCCESS_RUN_DIR="$(latest_run_dir)"
SUCCESS_SUMMARY="${SUCCESS_RUN_DIR}/summary.json"
assert_contains "${SUCCESS_OUTPUT}" "[security-gate] PASS: Oberon4 security verification satisfied. run_dir=${SUCCESS_RUN_DIR}"
[[ "$(summary_value "${SUCCESS_SUMMARY}" status)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.secret_scan)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.sast)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.dependency_audit)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.dependency_policy)" == "pass" ]] || exit 1
[[ -f "${SUCCESS_RUN_DIR}/gitleaks-report.json" ]] || { echo "missing gitleaks-report.json" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/semgrep-report.json" ]] || { echo "missing semgrep-report.json" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/cargo-audit-report.json" ]] || { echo "missing cargo-audit-report.json" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/cargo-deny-report.json" ]] || { echo "missing cargo-deny-report.json" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/commands.txt" ]] || { echo "missing commands.txt" >&2; exit 1; }
assert_contains "${ORDER_FILE}" "secret_scan"
assert_contains "${ORDER_FILE}" "sast"
assert_contains "${ORDER_FILE}" "dependency_audit"
assert_contains "${ORDER_FILE}" "dependency_policy"

MISSING_TOOL_OUTPUT="${TMP_DIR}/missing-tool.out"
if env PATH="/usr/bin:/bin" SECURITY_GATE_LOG_DIR="${RUN_ROOT}" GITLEAKS_CONFIG_PATH="${TMP_DIR}/gitleaks.toml" GITLEAKS_BASELINE_PATH="${TMP_DIR}/gitleaks-baseline.json" bash "${SECURITY_GATE_SCRIPT}" >"${MISSING_TOOL_OUTPUT}" 2>&1; then
  echo "expected missing-tool failure" >&2
  exit 1
fi
assert_contains "${MISSING_TOOL_OUTPUT}" "[security-gate] stage=secret_scan status=fail missing_tool=gitleaks"

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
if rg -n --fixed-strings "dependency_policy" "${ORDER_FILE}" >/dev/null; then
  echo "dependency_policy should be skipped" >&2
  exit 1
fi

BASELINE_OUTPUT="${TMP_DIR}/baseline.out"
run_gate "${BASELINE_OUTPUT}" MOCK_SECRET_SCAN_NEW_FINDING=0
assert_contains "${BASELINE_OUTPUT}" "[security-gate] PASS: Oberon4 security verification satisfied."

NEW_FINDING_OUTPUT="${TMP_DIR}/new-finding.out"
if run_gate "${NEW_FINDING_OUTPUT}" MOCK_SECRET_SCAN_NEW_FINDING=1; then
  echo "expected new-finding failure" >&2
  exit 1
fi
NEW_FINDING_RUN_DIR="$(latest_run_dir)"
NEW_FINDING_SUMMARY="${NEW_FINDING_RUN_DIR}/summary.json"
[[ "$(summary_value "${NEW_FINDING_SUMMARY}" status)" == "fail" ]] || exit 1
[[ "$(summary_value "${NEW_FINDING_SUMMARY}" stages.secret_scan)" == "fail" ]] || exit 1
assert_contains "${NEW_FINDING_OUTPUT}" "[security-gate] stage=secret_scan status=fail run_dir=${NEW_FINDING_RUN_DIR} log=${NEW_FINDING_RUN_DIR}/secret_scan.log report=${NEW_FINDING_RUN_DIR}/gitleaks-report.json"
assert_contains "${NEW_FINDING_RUN_DIR}/secret_scan.log" "gitleaks synthetic new finding"
assert_contains "${NEW_FINDING_RUN_DIR}/gitleaks-report.json" "new-fingerprint"

POLICY_FAIL_OUTPUT="${TMP_DIR}/policy-fail.out"
if run_gate "${POLICY_FAIL_OUTPUT}" MOCK_DEPENDENCY_POLICY_FAIL=1; then
  echo "expected dependency-policy failure" >&2
  exit 1
fi
POLICY_FAIL_RUN_DIR="$(latest_run_dir)"
POLICY_FAIL_SUMMARY="${POLICY_FAIL_RUN_DIR}/summary.json"
[[ "$(summary_value "${POLICY_FAIL_SUMMARY}" stages.dependency_policy)" == "fail" ]] || exit 1
assert_contains "${POLICY_FAIL_OUTPUT}" "[security-gate] stage=dependency_policy status=fail run_dir=${POLICY_FAIL_RUN_DIR} log=${POLICY_FAIL_RUN_DIR}/dependency_policy.log report=${POLICY_FAIL_RUN_DIR}/cargo-deny-report.json"
assert_contains "${POLICY_FAIL_RUN_DIR}/dependency_policy.log" "cargo-deny synthetic failure"

WRAPPER_OUTPUT="${TMP_DIR}/wrapper.out"
env PATH="${MOCK_BIN}:$PATH" \
  ORDER_FILE="${ORDER_FILE}" \
  SECURITY_GATE_LOG_DIR="${RUN_ROOT}" \
  GITLEAKS_CONFIG_PATH="${TMP_DIR}/gitleaks.toml" \
  GITLEAKS_BASELINE_PATH="${TMP_DIR}/gitleaks-baseline.json" \
  SEMGREP_CONFIG_PATH="${TMP_DIR}/semgrep.yml" \
  CARGO_DENY_CONFIG_PATH="${TMP_DIR}/deny.toml" \
  EXPECTED_SEMGREP_CONFIG="${TMP_DIR}/semgrep.yml" \
  bash "${WRAPPER_SCRIPT}" >"${WRAPPER_OUTPUT}" 2>&1
assert_contains "${WRAPPER_OUTPUT}" "scripts/security-gate.sh is deprecated"
assert_contains "${WRAPPER_OUTPUT}" "[security-gate] PASS: Oberon4 security verification satisfied."

echo "test-security-gate.sh: PASS"
