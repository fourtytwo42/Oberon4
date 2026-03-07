#!/usr/bin/env bash
set -euo pipefail

: "${OBERON4_LIVE_BASE_URL:?set OBERON4_LIVE_BASE_URL to the live API base}"

CURRENT_STAGE="bootstrap"
trap 'status=$?; printf "\nLIVE_SMOKE_RESULT failed\n"; printf "LIVE_SMOKE_STAGE %s\n" "$CURRENT_STAGE"; exit "$status"' ERR

export TMPDIR="${TMPDIR:-/home/hendo420/Oberon4/.tmp}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/hendo420/Oberon4/target}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export RUSTFLAGS="${RUSTFLAGS:--C debuginfo=0}"

BASE_URL="${OBERON4_LIVE_BASE_URL%/}"
BASE_URL="${BASE_URL%/v1}"
OPENAI_MODEL="${OBERON4_LIVE_OPENAI_MODEL:-gpt-5.1-codex-mini}"
API_KEY_ENV="${OBERON4_LIVE_API_KEY_ENV:-}"

printf 'Using base URL: %s\n' "$BASE_URL"
printf 'OpenAI model: %s\n' "$OPENAI_MODEL"
if [ -n "$API_KEY_ENV" ]; then
  printf 'API key env: %s\n' "$API_KEY_ENV"
fi

echo
echo "== Build real CLI and daemon binaries =="
CURRENT_STAGE="build_binaries"
cargo build -p oberon4-daemon -p oberon4-cli

echo
echo "== Run daemon-backed live CLI smoke =="
CURRENT_STAGE="daemon_cli_live_smoke"
OBERON4_LIVE_HOTSPOT=1 \
OBERON4_LIVE_BASE_URL="$BASE_URL" \
OBERON4_LIVE_OPENAI_MODEL="$OPENAI_MODEL" \
OBERON4_LIVE_API_KEY_ENV="$API_KEY_ENV" \
cargo test -p oberon4-cli --test live_daemon -- --ignored --nocapture

CURRENT_STAGE="complete"
echo
echo "LIVE_SMOKE_RESULT passed"
