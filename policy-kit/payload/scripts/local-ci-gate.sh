#!/usr/bin/env bash
set -euo pipefail

source scripts/lib/common.sh
load_hooks_env

run_local_ci="${HOOKS_RUN_LOCAL_CI_ON_PUSH:-1}"
if [[ "$run_local_ci" != "1" ]]; then
  exit 0
fi

strict="${HOOKS_LOCAL_CI_STRICT:-1}"

require_smoke="${HOOKS_LOCAL_CI_REQUIRE_SMOKE:-1}"
require_test="${HOOKS_LOCAL_CI_REQUIRE_TEST:-1}"
require_build="${HOOKS_LOCAL_CI_REQUIRE_BUILD:-1}"

smoke_cmd="${HOOKS_LOCAL_SMOKE_CMD:-}"
test_cmd="${HOOKS_LOCAL_TEST_CMD:-}"
build_cmd="${HOOKS_LOCAL_BUILD_CMD:-}"

run_step() {
  local label="$1"
  local cmd="$2"
  local required="$3"

  if [[ -z "${cmd//[[:space:]]/}" ]]; then
    if [[ "$required" == "1" ]]; then
      echo "Local CI gate failed: missing command for ${label}." >&2
      return 1
    fi
    echo "Local CI gate: ${label} skipped (no command configured)."
    return 0
  fi

  echo "Local CI gate: running ${label}: ${cmd}"
  if ! bash -lc "$cmd"; then
    echo "Local CI gate failed at ${label}." >&2
    return 1
  fi

  return 0
}

if [[ "$strict" == "1" ]]; then
  run_step "smoke" "$smoke_cmd" "$require_smoke"
  run_step "tests" "$test_cmd" "$require_test"
  run_step "build" "$build_cmd" "$require_build"
else
  run_step "smoke" "$smoke_cmd" 0
  run_step "tests" "$test_cmd" 0
  run_step "build" "$build_cmd" 0
fi

echo "Local CI gate: OK"
