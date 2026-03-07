#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

: "${MAIN_APP_URL:=http://127.0.0.1:11434}"
: "${OUTPUT_DIR:=${ROOT_DIR}/artifacts/ui-screenshots}"
: "${SCREEN_WIDTH:=1720}"
: "${SCREEN_HEIGHT:=1080}"

mkdir -p "${OUTPUT_DIR}"

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
console_png="${OUTPUT_DIR}/console-${timestamp}.png"
endpoints_png="${OUTPUT_DIR}/endpoints-${timestamp}.png"
console_latest_png="${OUTPUT_DIR}/console-latest.png"
endpoints_latest_png="${OUTPUT_DIR}/endpoints-latest.png"
manifest_json="${OUTPUT_DIR}/manifest.json"

chrome_bin=""
if command -v google-chrome >/dev/null 2>&1; then
  chrome_bin="google-chrome"
elif command -v chromium-browser >/dev/null 2>&1; then
  chrome_bin="chromium-browser"
elif command -v chromium >/dev/null 2>&1; then
  chrome_bin="chromium"
fi

capture_with_chrome() {
  local url="$1"
  local out="$2"
  "${chrome_bin}" \
    --headless \
    --disable-gpu \
    --window-size="${SCREEN_WIDTH},${SCREEN_HEIGHT}" \
    --screenshot="${out}" \
    "${url}" >/dev/null 2>&1
}

capture_with_playwright() {
  local url="$1"
  local out="$2"
  node -e '
const fs = require("fs");
const { chromium } = require("playwright");
const url = process.argv[1];
const out = process.argv[2];
(async () => {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: Number(process.env.SCREEN_WIDTH || 1720), height: Number(process.env.SCREEN_HEIGHT || 1080) }});
  await page.goto(url, { waitUntil: "networkidle" });
  await page.screenshot({ path: out, fullPage: true });
  await browser.close();
})().catch((err) => {
  console.error(err?.stack || String(err));
  process.exit(1);
});
' "$url" "$out"
}

sha256_file() {
  local input="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${input}" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${input}" | awk '{print $1}'
  else
    openssl dgst -sha256 -r "${input}" | awk '{print $1}'
  fi
}

have_playwright=0
if (
  cd "${ROOT_DIR}/apps/website"
  node -e 'require.resolve("playwright")' >/dev/null 2>&1
); then
  have_playwright=1
fi

echo "[screens] capturing console screenshots..."
if [ -n "${chrome_bin}" ]; then
  capture_with_chrome "${MAIN_APP_URL}/console" "${console_png}"
  capture_with_chrome "${MAIN_APP_URL}/v1/endpoints" "${endpoints_png}"
elif [ "${have_playwright}" = "1" ]; then
  (
    cd "${ROOT_DIR}/apps/website"
    capture_with_playwright "${MAIN_APP_URL}/console" "${console_png}"
    capture_with_playwright "${MAIN_APP_URL}/v1/endpoints" "${endpoints_png}"
  )
else
  echo "[screens] ERROR: no headless browser detected."
  echo "[screens] Install one of: google-chrome / chromium / chromium-browser"
  echo "[screens] or install Playwright for apps/website"
  exit 1
fi

cp "${console_png}" "${console_latest_png}"
cp "${endpoints_png}" "${endpoints_latest_png}"

console_sha="$(sha256_file "${console_latest_png}")"
endpoints_sha="$(sha256_file "${endpoints_latest_png}")"

cat > "${manifest_json}" <<JSON
{
  "captured_at_utc": "${timestamp}",
  "artifacts": [
    {
      "path": "console-latest.png",
      "sha256": "${console_sha}"
    },
    {
      "path": "endpoints-latest.png",
      "sha256": "${endpoints_sha}"
    }
  ]
}
JSON

echo "[screens] wrote:"
echo "  - ${console_png}"
echo "  - ${endpoints_png}"
echo "  - ${console_latest_png}"
echo "  - ${endpoints_latest_png}"
echo "  - ${manifest_json}"
