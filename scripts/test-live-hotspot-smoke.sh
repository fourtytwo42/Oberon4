#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SMOKE_SCRIPT="${ROOT_DIR}/scripts/live-hotspot-smoke.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
mkdir -p "${MOCK_BIN}"

cat > "${MOCK_BIN}/curl" <<'SH'
#!/usr/bin/bash
set -euo pipefail
url="${@: -1}"
version_response="${MOCK_VERSION_RESPONSE:-}"
models_response="${MOCK_MODELS_RESPONSE:-}"
auth_response="${MOCK_AUTH_STATUS_RESPONSE:-}"
diagnostics_response="${MOCK_DIAGNOSTICS_RESPONSE:-}"
case "$url" in
  */v1/version)
    if [[ -n "${version_response}" ]]; then
      printf '%s\n' "${version_response}"
    else
      printf '%s\n' '{"version":"test"}'
    fi
    ;;
  */v1/models)
    if [[ -n "${models_response}" ]]; then
      printf '%s\n' "${models_response}"
    else
      printf '%s\n' '{"data":[{"id":"gpt-5.1-codex-mini"}]}'
    fi
    ;;
  */v1/auth/status)
    if [[ -n "${auth_response}" ]]; then
      printf '%s\n' "${auth_response}"
    else
      printf '%s\n' '{"entitled":true}'
    fi
    ;;
  */v1/diagnostics)
    if [[ -n "${diagnostics_response}" ]]; then
      printf '%s\n' "${diagnostics_response}"
    else
      printf '%s\n' '{"requests":1}'
    fi
    ;;
  */v1/chat/completions)
    printf '%s\n' '{"choices":[{"message":{"role":"assistant","content":"live endpoint ok"}}]}'
    ;;
  */v1/messages)
    printf '%s\n' '{"content":[{"type":"text","text":"anthropic path ok"}]}'
    ;;
  *)
    echo "unexpected curl URL: $url" >&2
    exit 1
    ;;
esac
SH
chmod +x "${MOCK_BIN}/curl"

cat > "${MOCK_BIN}/cargo" <<'SH'
#!/usr/bin/bash
set -euo pipefail
if [[ "${1:-}" == "test" ]]; then
  echo "cargo test $*"
  exit 0
fi
echo "unexpected cargo invocation: $*" >&2
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

run_smoke() {
  local output_file="$1"
  shift
  env PATH="${MOCK_BIN}:$PATH" \
    OBERON4_LIVE_BASE_URL="http://example.test/v1" \
    OBERON4_LIVE_OPENAI_MODEL="gpt-5.1-codex-mini" \
    "$@" bash "${SMOKE_SCRIPT}" >"${output_file}" 2>&1
}

SUCCESS_OUTPUT="${TMP_DIR}/success.out"
run_smoke "${SUCCESS_OUTPUT}"
assert_contains "${SUCCESS_OUTPUT}" "== GET /v1/version =="
assert_contains "${SUCCESS_OUTPUT}" "== GET /v1/models =="
assert_contains "${SUCCESS_OUTPUT}" "== GET /v1/auth/status =="
assert_contains "${SUCCESS_OUTPUT}" "== GET /v1/diagnostics =="
assert_contains "${SUCCESS_OUTPUT}" "== Rust live provider test =="

MODELS_FAIL_OUTPUT="${TMP_DIR}/models-fail.out"
if run_smoke "${MODELS_FAIL_OUTPUT}" MOCK_MODELS_RESPONSE='{"data":[{"id":"other-model"}]}'; then
  echo "expected models validation failure" >&2
  exit 1
fi
assert_contains "${MODELS_FAIL_OUTPUT}" "/v1/models missing required model: gpt-5.1-codex-mini"

AUTH_FAIL_OUTPUT="${TMP_DIR}/auth-fail.out"
if run_smoke "${AUTH_FAIL_OUTPUT}" MOCK_AUTH_STATUS_RESPONSE='not-json'; then
  echo "expected auth JSON validation failure" >&2
  exit 1
fi
assert_contains "${AUTH_FAIL_OUTPUT}" "/v1/auth/status returned invalid JSON"

DIAGNOSTICS_FAIL_OUTPUT="${TMP_DIR}/diagnostics-fail.out"
if run_smoke "${DIAGNOSTICS_FAIL_OUTPUT}" MOCK_DIAGNOSTICS_RESPONSE='not-json'; then
  echo "expected diagnostics JSON validation failure" >&2
  exit 1
fi
assert_contains "${DIAGNOSTICS_FAIL_OUTPUT}" "/v1/diagnostics returned invalid JSON"

echo "test-live-hotspot-smoke.sh: PASS"
