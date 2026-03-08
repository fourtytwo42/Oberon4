#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

: "${BOOTSTRAP_BIN_DIR:=${HOME}/.local/bin}"
: "${BOOTSTRAP_CARGO_INSTALL_ROOT:=${HOME}/.cargo}"
: "${BOOTSTRAP_TMP_DIR:=$(mktemp -d)}"
: "${BOOTSTRAP_GITLEAKS_VERSION:=8.30.0}"
: "${BOOTSTRAP_GITLEAKS_URL:=https://github.com/gitleaks/gitleaks/releases/download/v${BOOTSTRAP_GITLEAKS_VERSION}/gitleaks_${BOOTSTRAP_GITLEAKS_VERSION}_linux_x64.tar.gz}"
: "${BOOTSTRAP_GET_PIP_URL:=https://bootstrap.pypa.io/get-pip.py}"
: "${BOOTSTRAP_SEMGREP_SPEC:=semgrep}"
: "${BOOTSTRAP_CARGO_AUDIT_SPEC:=cargo-audit}"
: "${BOOTSTRAP_CARGO_DENY_VERSION:=0.19.0}"
: "${BOOTSTRAP_CARGO_DENY_URL:=}"

cleanup() {
  if [[ -d "${BOOTSTRAP_TMP_DIR}" && "${BOOTSTRAP_TMP_DIR}" == /tmp/* ]]; then
    rm -rf "${BOOTSTRAP_TMP_DIR}"
  fi
}
trap cleanup EXIT

mkdir -p "${BOOTSTRAP_BIN_DIR}" "${BOOTSTRAP_CARGO_INSTALL_ROOT}/bin"

log() {
  echo "[bootstrap-security-tools] $*"
}

join_with_colon() {
  local joined=""
  local part
  for part in "$@"; do
    if [[ -z "${joined}" ]]; then
      joined="${part}"
    else
      joined="${joined}:${part}"
    fi
  done
  printf '%s' "${joined}"
}

ensure_on_path_note() {
  local export_parts=()
  [[ ":${PATH}:" != *":${BOOTSTRAP_BIN_DIR}:"* ]] && export_parts+=("${BOOTSTRAP_BIN_DIR}")
  [[ ":${PATH}:" != *":${BOOTSTRAP_CARGO_INSTALL_ROOT}/bin:"* ]] && export_parts+=("${BOOTSTRAP_CARGO_INSTALL_ROOT}/bin")
  if (( ${#export_parts[@]} > 0 )); then
    printf '[bootstrap-security-tools] add to PATH: export PATH="%s:$PATH"\n' "$(join_with_colon "${export_parts[@]}")"
  fi
}

install_gitleaks() {
  if command -v gitleaks >/dev/null 2>&1; then
    log "gitleaks already available at $(command -v gitleaks)"
    return
  fi
  local archive_path="${BOOTSTRAP_TMP_DIR}/gitleaks.tar.gz"
  local extract_dir="${BOOTSTRAP_TMP_DIR}/gitleaks"
  mkdir -p "${extract_dir}"
  log "installing gitleaks from ${BOOTSTRAP_GITLEAKS_URL}"
  curl -fsSL "${BOOTSTRAP_GITLEAKS_URL}" -o "${archive_path}"
  tar -xzf "${archive_path}" -C "${extract_dir}"
  local candidate
  candidate="$(find "${extract_dir}" -maxdepth 2 -type f -name gitleaks | head -n 1)"
  [[ -n "${candidate}" ]] || { log "failed to locate gitleaks binary in downloaded archive"; exit 1; }
  install "${candidate}" "${BOOTSTRAP_BIN_DIR}/gitleaks"
  log "installed gitleaks to ${BOOTSTRAP_BIN_DIR}/gitleaks"
}

ensure_pip() {
  if python3 -m pip --version >/dev/null 2>&1; then
    return
  fi
  local get_pip_path="${BOOTSTRAP_TMP_DIR}/get-pip.py"
  log "installing pip via ${BOOTSTRAP_GET_PIP_URL}"
  curl -fsSL "${BOOTSTRAP_GET_PIP_URL}" -o "${get_pip_path}"
  python3 "${get_pip_path}" --user --break-system-packages
}

install_semgrep() {
  if command -v semgrep >/dev/null 2>&1; then
    log "semgrep already available at $(command -v semgrep)"
    return
  fi
  ensure_pip
  log "installing semgrep via python3 -m pip"
  python3 -m pip install --user --break-system-packages "${BOOTSTRAP_SEMGREP_SPEC}"
  local candidate="${BOOTSTRAP_BIN_DIR}/semgrep"
  [[ -x "${candidate}" ]] || { log "semgrep installation completed but binary was not found at ${candidate}"; exit 1; }
  log "installed semgrep to ${candidate}"
}

install_cargo_audit() {
  if command -v cargo-audit >/dev/null 2>&1; then
    log "cargo-audit already available at $(command -v cargo-audit)"
    return
  fi
  log "installing cargo-audit into ${BOOTSTRAP_CARGO_INSTALL_ROOT}/bin"
  cargo install --root "${BOOTSTRAP_CARGO_INSTALL_ROOT}" --locked "${BOOTSTRAP_CARGO_AUDIT_SPEC}"
  local candidate="${BOOTSTRAP_CARGO_INSTALL_ROOT}/bin/cargo-audit"
  [[ -x "${candidate}" ]] || { log "cargo-audit installation completed but binary was not found at ${candidate}"; exit 1; }
  log "installed cargo-audit to ${candidate}"
}

install_cargo_deny() {
  if command -v cargo-deny >/dev/null 2>&1; then
    local existing_path existing_version
    existing_path="$(command -v cargo-deny)"
    existing_version="$(cargo-deny --version 2>/dev/null | awk '{print $2}')"
    if [[ "${existing_version}" == "${BOOTSTRAP_CARGO_DENY_VERSION}" ]]; then
      log "cargo-deny already available at ${existing_path}"
      return
    fi
    log "replacing cargo-deny ${existing_version:-unknown} at ${existing_path} with ${BOOTSTRAP_CARGO_DENY_VERSION}"
  fi

  local platform=""
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) platform="x86_64-unknown-linux-musl" ;;
    Linux-aarch64) platform="aarch64-unknown-linux-musl" ;;
    *)
      log "cargo-deny bootstrap is unsupported on $(uname -s)-$(uname -m); set BOOTSTRAP_CARGO_DENY_URL explicitly"
      exit 1
      ;;
  esac

  local download_url="${BOOTSTRAP_CARGO_DENY_URL}"
  if [[ -z "${download_url}" ]]; then
    download_url="https://github.com/EmbarkStudios/cargo-deny/releases/download/${BOOTSTRAP_CARGO_DENY_VERSION}/cargo-deny-${BOOTSTRAP_CARGO_DENY_VERSION}-${platform}.tar.gz"
  fi

  local archive_path="${BOOTSTRAP_TMP_DIR}/cargo-deny.tar.gz"
  local extract_dir="${BOOTSTRAP_TMP_DIR}/cargo-deny"
  mkdir -p "${extract_dir}"

  log "installing cargo-deny from ${download_url}"
  curl -fsSL "${download_url}" -o "${archive_path}"
  tar -xzf "${archive_path}" -C "${extract_dir}"

  local downloaded_binary
  downloaded_binary="$(find "${extract_dir}" -maxdepth 2 -type f -name cargo-deny | head -n 1)"
  [[ -n "${downloaded_binary}" ]] || { log "failed to locate cargo-deny binary in downloaded archive"; exit 1; }
  install "${downloaded_binary}" "${BOOTSTRAP_CARGO_INSTALL_ROOT}/bin/cargo-deny"

  local candidate="${BOOTSTRAP_CARGO_INSTALL_ROOT}/bin/cargo-deny"
  [[ -x "${candidate}" ]] || { log "cargo-deny installation completed but binary was not found at ${candidate}"; exit 1; }
  log "installed cargo-deny to ${candidate}"
}

install_gitleaks
install_semgrep
install_cargo_audit
install_cargo_deny
ensure_on_path_note
log "PASS"
