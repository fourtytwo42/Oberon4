#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"

mkdir -p "${DIST_DIR}"

workspace_version() {
  python3 - <<'PY'
import re, pathlib
txt = pathlib.Path("Cargo.toml").read_text(encoding="utf-8")
m = re.search(r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"', txt)
if not m:
  raise SystemExit("failed to find [workspace.package] version in Cargo.toml")
print(m.group(1))
PY
}

host_arch() {
  local arch
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) echo "x86_64" ;;
    aarch64|arm64) echo "arm64" ;;
    *) echo "$arch" ;;
  esac
}

download_cloudflared_linux() {
  local arch url out tmp
  arch="$(host_arch)"
  case "$arch" in
    x86_64) url="https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64" ;;
    arm64) url="https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64" ;;
    *) echo "unsupported linux arch for cloudflared download: ${arch}" >&2; return 1 ;;
  esac
  out="$1"
  tmp="${out}.tmp"
  curl -fsSL -o "${tmp}" "${url}"
  chmod +x "${tmp}"
  mv "${tmp}" "${out}"
}

download_cloudflared_macos() {
  local arch url out tmp tmpdir
  arch="$(host_arch)"
  case "$arch" in
    x86_64) url="https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-amd64.tgz" ;;
    arm64) url="https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-arm64.tgz" ;;
    *) echo "unsupported macOS arch for cloudflared download: ${arch}" >&2; return 1 ;;
  esac
  out="$1"
  tmp="${out}.tgz.tmp"
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "${tmpdir}"' RETURN
  curl -fsSL -o "${tmp}" "${url}"
  tar -C "${tmpdir}" -xzf "${tmp}"
  if [ ! -f "${tmpdir}/cloudflared" ]; then
    echo "cloudflared binary not found in downloaded archive" >&2
    rm -f "${tmp}"
    return 1
  fi
  chmod +x "${tmpdir}/cloudflared"
  mv "${tmpdir}/cloudflared" "${out}"
  rm -f "${tmp}"
}
