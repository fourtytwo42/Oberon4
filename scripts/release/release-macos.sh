#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

cd "${ROOT_DIR}"

VERSION="$(workspace_version)"

echo "Building llmhotspot (release) for macOS..."
echo "Note: for a universal build you'll need to build both arch targets and lipo them."
echo "This script produces a single-arch artifact based on the host architecture."

cargo build -q -p llmhotspot --release --locked

ARCH="$(host_arch)"
BIN_DIR="${DIST_DIR}/llmhotspot-v${VERSION}-macos-${ARCH}"
rm -rf "${BIN_DIR}"
mkdir -p "${BIN_DIR}"

cp -f "${ROOT_DIR}/target/release/llmhotspot" "${BIN_DIR}/llmhotspot"

echo "Bundling cloudflared..."
download_cloudflared_macos "${BIN_DIR}/cloudflared"

cat >"${BIN_DIR}/README.txt" <<'TXT'
LLM Hotspot (v2) - macOS

Run:
  ./llmhotspot serve

Then open:
  http://localhost:11434/console

Notes:
- cloudflared is bundled as ./cloudflared and is managed by the app when remote mode is enabled.
- For auto-updates on macOS, codesign + notarization are strongly recommended.
TXT

TARBALL="${DIST_DIR}/llmhotspot-v${VERSION}-macos-${ARCH}.tar.gz"
echo "Packaging ${TARBALL}"
tar -C "${DIST_DIR}" -czf "${TARBALL}" "$(basename "${BIN_DIR}")"

echo "Done:"
echo "  ${TARBALL}"
