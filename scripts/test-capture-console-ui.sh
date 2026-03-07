#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CAPTURE_SCRIPT="${ROOT_DIR}/scripts/capture-console-ui.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
OUTPUT_DIR="${TMP_DIR}/artifacts"
mkdir -p "${MOCK_BIN}" "${OUTPUT_DIR}"

cat > "${MOCK_BIN}/google-chrome" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
for arg in "$@"; do
  if [[ "${arg}" == --screenshot=* ]]; then
    screenshot_path="${arg#--screenshot=}"
    printf 'synthetic screenshot for %s\n' "$*" > "${screenshot_path}"
    exit 0
  fi
done
echo "missing --screenshot argument" >&2
exit 1
SH
chmod +x "${MOCK_BIN}/google-chrome"

PATH="${MOCK_BIN}:$PATH" OUTPUT_DIR="${OUTPUT_DIR}" MAIN_APP_URL="http://127.0.0.1:65535" bash "${CAPTURE_SCRIPT}" > "${TMP_DIR}/capture.out"

timestamped_console_count="$(find "${OUTPUT_DIR}" -maxdepth 1 -name 'console-*.png' ! -name 'console-latest.png' | wc -l | tr -d ' ')"
timestamped_endpoints_count="$(find "${OUTPUT_DIR}" -maxdepth 1 -name 'endpoints-*.png' ! -name 'endpoints-latest.png' | wc -l | tr -d ' ')"

[[ "${timestamped_console_count}" == "1" ]]
[[ "${timestamped_endpoints_count}" == "1" ]]
[[ -f "${OUTPUT_DIR}/console-latest.png" ]]
[[ -f "${OUTPUT_DIR}/endpoints-latest.png" ]]
[[ -f "${OUTPUT_DIR}/manifest.json" ]]

cmp -s "${OUTPUT_DIR}/console-latest.png" "$(find "${OUTPUT_DIR}" -maxdepth 1 -name 'console-*.png' ! -name 'console-latest.png' | head -n 1)"
cmp -s "${OUTPUT_DIR}/endpoints-latest.png" "$(find "${OUTPUT_DIR}" -maxdepth 1 -name 'endpoints-*.png' ! -name 'endpoints-latest.png' | head -n 1)"

if ! rg -n --fixed-strings '"path": "console-latest.png"' "${OUTPUT_DIR}/manifest.json" >/dev/null; then
  echo "manifest missing console stable artifact" >&2
  exit 1
fi

if ! rg -n --fixed-strings '"path": "endpoints-latest.png"' "${OUTPUT_DIR}/manifest.json" >/dev/null; then
  echo "manifest missing endpoints stable artifact" >&2
  exit 1
fi

echo "test-capture-console-ui.sh: PASS"
