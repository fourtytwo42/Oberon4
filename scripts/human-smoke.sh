#!/usr/bin/env bash
set -euo pipefail

export TMPDIR="${TMPDIR:-/home/hendo420/Oberon4/.tmp}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/hendo420/Oberon4/target}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
export RUSTFLAGS="${RUSTFLAGS:--C debuginfo=0}"

mkdir -p "$TMPDIR"

echo "[1/4] fmt"
cargo fmt --all -- --check

echo "[2/4] clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "[3/4] workspace tests"
cargo test --workspace

echo "[4/4] e2e smoke"
cargo test -p oberon4-cli --test end_to_end

echo "human test preflight complete"
