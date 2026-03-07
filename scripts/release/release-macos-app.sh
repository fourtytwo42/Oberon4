#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_common.sh"

cd "${ROOT_DIR}"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "release-macos-app.sh must run on macOS (Darwin)." >&2
  exit 1
fi

VERSION="$(workspace_version)"
ARCH="$(host_arch)"

echo "Building llmhotspot (release) for macOS/${ARCH}..."
cargo build -q -p llmhotspot --release --locked

BUNDLE_ROOT="${DIST_DIR}/LLM Hotspot.app"
CONTENTS_DIR="${BUNDLE_ROOT}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"

rm -rf "${BUNDLE_ROOT}"
mkdir -p "${MACOS_DIR}"

cp -f "${ROOT_DIR}/target/release/llmhotspot" "${MACOS_DIR}/llmhotspot"
echo "Bundling cloudflared..."
download_cloudflared_macos "${MACOS_DIR}/cloudflared"

cat >"${MACOS_DIR}/LLMHotspotLauncher" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "${SCRIPT_DIR}/llmhotspot" serve
SH
chmod +x "${MACOS_DIR}/LLMHotspotLauncher" "${MACOS_DIR}/llmhotspot" "${MACOS_DIR}/cloudflared"

cat >"${CONTENTS_DIR}/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>LLM Hotspot</string>
  <key>CFBundleDisplayName</key>
  <string>LLM Hotspot</string>
  <key>CFBundleIdentifier</key>
  <string>com.llmhotspot.app</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleExecutable</key>
  <string>LLMHotspotLauncher</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
</dict>
</plist>
EOF

echo "Created app bundle:"
echo "  ${BUNDLE_ROOT}"

if ! command -v hdiutil >/dev/null 2>&1; then
  echo "hdiutil not found; skipping DMG creation." >&2
  exit 0
fi

DMG_PATH="${DIST_DIR}/LLMHotspot-v${VERSION}-macos-${ARCH}.dmg"
rm -f "${DMG_PATH}"
echo "Creating DMG ${DMG_PATH}"
hdiutil create -volname "LLM Hotspot" -srcfolder "${BUNDLE_ROOT}" -ov -format UDZO "${DMG_PATH}" >/dev/null

echo "Done:"
echo "  ${BUNDLE_ROOT}"
echo "  ${DMG_PATH}"

