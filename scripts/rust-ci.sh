#!/usr/bin/env bash
set -euo pipefail

export TMPDIR="${TMPDIR:-/home/hendo420/Oberon4/.tmp}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/home/hendo420/Oberon4/target}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export RUSTFLAGS="${RUSTFLAGS:--C debuginfo=0}"

mkdir -p "$TMPDIR"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
