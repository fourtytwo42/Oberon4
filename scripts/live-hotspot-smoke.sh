#!/usr/bin/env bash
set -euo pipefail

: "${OBERON4_LIVE_BASE_URL:?set OBERON4_LIVE_BASE_URL to the LLM Hotspot API base}"

BASE_URL="${OBERON4_LIVE_BASE_URL%/}"
BASE_URL="${BASE_URL%/v1}"
OPENAI_MODEL="${OBERON4_LIVE_OPENAI_MODEL:-gpt-5.1-codex-mini}"
ANTHROPIC_MODEL="${OBERON4_LIVE_ANTHROPIC_MODEL:-claude-sonnet-4-6}"
TIMEOUT_SECS="${OBERON4_LIVE_TIMEOUT_SECS:-90}"
API_KEY_ENV="${OBERON4_LIVE_API_KEY_ENV:-}"

AUTH_ARGS=()
if [ -n "$API_KEY_ENV" ]; then
  API_KEY_VALUE="${!API_KEY_ENV:-}"
  if [ -z "$API_KEY_VALUE" ]; then
    echo "OBERON4_LIVE_API_KEY_ENV is set to '$API_KEY_ENV' but that env var is empty" >&2
    exit 1
  fi
  AUTH_ARGS=(-H "authorization: Bearer $API_KEY_VALUE")
fi

printf 'Using base URL: %s\n' "$BASE_URL"
printf 'OpenAI model: %s\n' "$OPENAI_MODEL"
printf 'Anthropic model: %s\n' "$ANTHROPIC_MODEL"
if [ -n "$API_KEY_ENV" ]; then
  printf 'API key env: %s\n' "$API_KEY_ENV"
fi

validate_json_field() {
  local label="$1"
  local payload="$2"
  local js="$3"
  LABEL="$label" PAYLOAD="$payload" JS_EXPR="$js" node <<'NODE'
const label = process.env.LABEL;
const payload = process.env.PAYLOAD;
const expr = process.env.JS_EXPR;

let value;
try {
  value = JSON.parse(payload);
} catch (err) {
  console.error(`${label} returned invalid JSON: ${err.message}`);
  process.exit(1);
}

const fn = new Function("value", expr);
const ok = fn(value);
if (!ok) {
  console.error(`${label} did not satisfy validation`);
  process.exit(1);
}
NODE
}

printf '\n== GET /v1/version ==\n'
VERSION_RESPONSE="$(curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "${AUTH_ARGS[@]}" \
  "$BASE_URL/v1/version")"
printf '%s' "$VERSION_RESPONSE"
validate_json_field "/v1/version" "$VERSION_RESPONSE" 'return typeof value.version === "string" && value.version.length > 0;'
printf '\n\n== GET /v1/models ==\n'
MODELS_RESPONSE="$(curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "${AUTH_ARGS[@]}" \
  "$BASE_URL/v1/models")"
printf '%s' "$MODELS_RESPONSE"
LABEL="/v1/models" PAYLOAD="$MODELS_RESPONSE" REQUIRED_MODEL="$OPENAI_MODEL" node <<'NODE'
const payload = process.env.PAYLOAD;
const requiredModel = process.env.REQUIRED_MODEL;

let value;
try {
  value = JSON.parse(payload);
} catch (err) {
  console.error(`/v1/models returned invalid JSON: ${err.message}`);
  process.exit(1);
}

const items = Array.isArray(value)
  ? value
  : Array.isArray(value.data)
    ? value.data
    : [];
const models = items
  .map((item) => {
    if (typeof item === "string") {
      return item;
    }
    if (item && typeof item === "object") {
      return item.id || item.model || null;
    }
    return null;
  })
  .filter(Boolean);

if (!models.includes(requiredModel)) {
  console.error(`/v1/models missing required model: ${requiredModel}`);
  process.exit(1);
}
NODE
printf '\n\n== GET /v1/auth/status ==\n'
AUTH_STATUS_RESPONSE="$(curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "${AUTH_ARGS[@]}" \
  "$BASE_URL/v1/auth/status")"
printf '%s' "$AUTH_STATUS_RESPONSE"
validate_json_field "/v1/auth/status" "$AUTH_STATUS_RESPONSE" 'return value !== null && typeof value === "object" && !Array.isArray(value);'
printf '\n\n== GET /v1/diagnostics ==\n'
DIAGNOSTICS_RESPONSE="$(curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "${AUTH_ARGS[@]}" \
  "$BASE_URL/v1/diagnostics")"
printf '%s' "$DIAGNOSTICS_RESPONSE"
validate_json_field "/v1/diagnostics" "$DIAGNOSTICS_RESPONSE" 'return value !== null && typeof value === "object" && !Array.isArray(value);'
printf '\n\n== OpenAI-compatible chat/completions ==\n'
curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "${AUTH_ARGS[@]}" \
  -H 'content-type: application/json' \
  -d "$(cat <<JSON
{"model":"$OPENAI_MODEL","stream":false,"max_completion_tokens":24,"messages":[{"role":"system","content":"Reply in under six words."},{"role":"user","content":"Say live endpoint ok"}]}
JSON
)" \
  "$BASE_URL/v1/chat/completions"
printf '\n\n== Anthropic-compatible messages ==\n'
curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "${AUTH_ARGS[@]}" \
  -H 'content-type: application/json' \
  -d "$(cat <<JSON
{"model":"$ANTHROPIC_MODEL","max_tokens":24,"messages":[{"role":"user","content":"Say anthropic path ok"}]}
JSON
)" \
  "$BASE_URL/v1/messages"
printf '\n\n== Rust live provider test ==\n'
OBERON4_LIVE_HOTSPOT=1 \
OBERON4_LIVE_BASE_URL="$BASE_URL" \
OBERON4_LIVE_OPENAI_MODEL="$OPENAI_MODEL" \
OBERON4_LIVE_API_KEY_ENV="$API_KEY_ENV" \
cargo test -p oberon4-provider-openai --test live_hotspot -- --ignored --nocapture
