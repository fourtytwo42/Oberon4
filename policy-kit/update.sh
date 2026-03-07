#!/usr/bin/env bash
set -euo pipefail

KIT_DIR="$(cd "$(dirname "$0")" && pwd)"

exec "$KIT_DIR/bootstrap.sh" --force "$@"
