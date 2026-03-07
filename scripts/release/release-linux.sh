#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

cd "${ROOT_DIR}"

VERSION="$(workspace_version)"
ARCH="$(host_arch)"

echo "Building llmhotspot (release) for linux/${ARCH}..."
cargo build -q -p llmhotspot --release --locked

BIN_DIR="${DIST_DIR}/llmhotspot-v${VERSION}-linux-${ARCH}"
rm -rf "${BIN_DIR}"
mkdir -p "${BIN_DIR}"

cp -f "${ROOT_DIR}/target/release/llmhotspot" "${BIN_DIR}/llmhotspot"

# Bundle cloudflared (linux only in this script).
echo "Bundling cloudflared..."
download_cloudflared_linux "${BIN_DIR}/cloudflared"

cat >"${BIN_DIR}/README.txt" <<'TXT'
LLM Hotspot (v2)

Run:
  ./llmhotspot serve

Then open:
  http://localhost:11434/console

Notes:
- cloudflared is bundled as ./cloudflared and is managed by the app when remote mode is enabled.
TXT

TARBALL="${DIST_DIR}/llmhotspot-v${VERSION}-linux-${ARCH}.tar.gz"
echo "Packaging ${TARBALL}"
tar -C "${DIST_DIR}" -czf "${TARBALL}" "$(basename "${BIN_DIR}")"

echo "Done:"
echo "  ${TARBALL}"

