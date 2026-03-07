#!/usr/bin/env bash
set -euo pipefail

: "${OBERON4_LIVE_BASE_URL:?set OBERON4_LIVE_BASE_URL to the LLM Hotspot API base}"

BASE_URL="${OBERON4_LIVE_BASE_URL%/}"
BASE_URL="${BASE_URL%/v1}"
OPENAI_MODEL="${OBERON4_LIVE_OPENAI_MODEL:-gpt-5.1-codex-mini}"
ANTHROPIC_MODEL="${OBERON4_LIVE_ANTHROPIC_MODEL:-claude-sonnet-4-6}"
TIMEOUT_SECS="${OBERON4_LIVE_TIMEOUT_SECS:-90}"

printf 'Using base URL: %s\n' "$BASE_URL"
printf 'OpenAI model: %s\n' "$OPENAI_MODEL"
printf 'Anthropic model: %s\n' "$ANTHROPIC_MODEL"

printf '\n== GET /v1/version ==\n'
curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "$BASE_URL/v1/version"
printf '\n\n== GET /v1/models ==\n'
curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  "$BASE_URL/v1/models"
printf '\n\n== OpenAI-compatible chat/completions ==\n'
curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
  -H 'content-type: application/json' \
  -d "$(cat <<JSON
{"model":"$OPENAI_MODEL","stream":false,"max_completion_tokens":24,"messages":[{"role":"system","content":"Reply in under six words."},{"role":"user","content":"Say live endpoint ok"}]}
JSON
)" \
  "$BASE_URL/v1/chat/completions"
printf '\n\n== Anthropic-compatible messages ==\n'
curl --fail --silent --show-error --max-time "$TIMEOUT_SECS" \
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
cargo test -p oberon4-provider-openai --test live_hotspot -- --ignored --nocapture
