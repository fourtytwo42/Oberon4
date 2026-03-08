#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BOOTSTRAP_SCRIPT="${ROOT_DIR}/scripts/bootstrap-security-tools.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
HOME_DIR="${TMP_DIR}/home"
mkdir -p "${MOCK_BIN}" "${HOME_DIR}"

cat > "${MOCK_BIN}/curl" <<'SH'
#!/usr/bin/bash
set -euo pipefail
if [[ "${1:-}" == "-fsSL" ]]; then
  source_path="$2"
  if [[ "${3:-}" == "-o" ]]; then
    target_path="$4"
    cp "${source_path#file://}" "${target_path}"
    exit 0
  fi
fi
echo "unexpected curl args: $*" >&2
exit 1
SH
chmod +x "${MOCK_BIN}/curl"

cat > "${MOCK_BIN}/python3" <<'SH'
#!/usr/bin/bash
set -euo pipefail
if [[ "${1:-}" == *"get-pip.py" ]]; then
  echo "mock get-pip ok"
  exit 0
fi
if [[ "${1:-}" == "-m" && "${2:-}" == "pip" && "${3:-}" == "--version" ]]; then
  if [[ "${MOCK_PIP_MISSING:-0}" == "1" ]]; then
    exit 1
  fi
  echo "pip 26.0"
  exit 0
fi
if [[ "${1:-}" == "-m" && "${2:-}" == "pip" && "${3:-}" == "install" ]]; then
  mkdir -p "${HOME}/.local/bin"
  cat > "${HOME}/.local/bin/semgrep" <<'INNER'
#!/usr/bin/bash
echo "semgrep 1.0"
INNER
  chmod +x "${HOME}/.local/bin/semgrep"
  echo "mock semgrep install ok"
  exit 0
fi
echo "unexpected python3 args: $*" >&2
exit 1
SH
chmod +x "${MOCK_BIN}/python3"

cat > "${MOCK_BIN}/cargo" <<'SH'
#!/usr/bin/bash
set -euo pipefail
if [[ "${1:-}" == "install" ]]; then
  root=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --root)
        root="$2"
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done
  mkdir -p "${root}/bin"
  cat > "${root}/bin/cargo-audit" <<'INNER'
#!/usr/bin/bash
echo "cargo-audit 1.0"
INNER
  chmod +x "${root}/bin/cargo-audit"
  echo "mock cargo-audit install ok"
  exit 0
fi
echo "unexpected cargo args: $*" >&2
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

GITLEAKS_STAGE_DIR="${TMP_DIR}/gitleaks-stage"
mkdir -p "${GITLEAKS_STAGE_DIR}"
cat > "${GITLEAKS_STAGE_DIR}/gitleaks" <<'SH'
#!/usr/bin/bash
echo "gitleaks 8.30.0"
SH
chmod +x "${GITLEAKS_STAGE_DIR}/gitleaks"
tar -czf "${TMP_DIR}/gitleaks.tar.gz" -C "${GITLEAKS_STAGE_DIR}" gitleaks
printf 'print(\"get-pip\")\n' > "${TMP_DIR}/get-pip.py"

SUCCESS_OUTPUT="${TMP_DIR}/success.out"
env PATH="${MOCK_BIN}:/usr/bin:/bin" \
  HOME="${HOME_DIR}" \
  BOOTSTRAP_GITLEAKS_URL="file://${TMP_DIR}/gitleaks.tar.gz" \
  BOOTSTRAP_GET_PIP_URL="file://${TMP_DIR}/get-pip.py" \
  bash "${BOOTSTRAP_SCRIPT}" >"${SUCCESS_OUTPUT}" 2>&1

[[ -x "${HOME_DIR}/.local/bin/gitleaks" ]] || { echo "missing installed gitleaks" >&2; exit 1; }
[[ -x "${HOME_DIR}/.local/bin/semgrep" ]] || { echo "missing installed semgrep" >&2; exit 1; }
[[ -x "${HOME_DIR}/.cargo/bin/cargo-audit" ]] || { echo "missing installed cargo-audit" >&2; exit 1; }
assert_contains "${SUCCESS_OUTPUT}" "[bootstrap-security-tools] installed gitleaks to ${HOME_DIR}/.local/bin/gitleaks"
assert_contains "${SUCCESS_OUTPUT}" "[bootstrap-security-tools] installed semgrep to ${HOME_DIR}/.local/bin/semgrep"
assert_contains "${SUCCESS_OUTPUT}" "[bootstrap-security-tools] installed cargo-audit to ${HOME_DIR}/.cargo/bin/cargo-audit"
assert_contains "${SUCCESS_OUTPUT}" "[bootstrap-security-tools] add to PATH: export PATH=\"${HOME_DIR}/.local/bin:${HOME_DIR}/.cargo/bin:\$PATH\""
assert_contains "${SUCCESS_OUTPUT}" "[bootstrap-security-tools] PASS"
[[ ! -e "${HOME_DIR}/.bashrc" ]] || { echo "bootstrap should not mutate shell profiles" >&2; exit 1; }

FAIL_OUTPUT="${TMP_DIR}/fail.out"
if env PATH="${MOCK_BIN}:/usr/bin:/bin" \
  HOME="${HOME_DIR}" \
  MOCK_PIP_MISSING=1 \
  BOOTSTRAP_GITLEAKS_URL="file://${TMP_DIR}/gitleaks.tar.gz" \
  BOOTSTRAP_GET_PIP_URL="file://${TMP_DIR}/missing-get-pip.py" \
  bash "${BOOTSTRAP_SCRIPT}" >"${FAIL_OUTPUT}" 2>&1; then
  echo "expected bootstrap failure for missing get-pip source" >&2
  exit 1
fi
assert_contains "${FAIL_OUTPUT}" "[bootstrap-security-tools] installing pip via file://${TMP_DIR}/missing-get-pip.py"

echo "test-bootstrap-security-tools.sh: PASS"
