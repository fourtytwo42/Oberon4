#!/usr/bin/env bash
set -euo pipefail

apply=0
scope="safe"

usage() {
  cat <<USAGE
Usage: $0 [--scope safe|python|node|all] [--apply]

Default is dry-run. Use --apply to perform changes.

Scopes:
  safe   Clear common temporary/runtime caches only (default)
  python Clear Python caches in addition to safe
  node   Clear Node caches/build outputs in addition to safe
  all    Apply safe + python + node
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --scope)
      scope="${2:-}"
      shift 2
      ;;
    --apply)
      apply=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 2
      ;;
  esac
done

case "$scope" in
  safe|python|node|all) ;;
  *)
    echo "Invalid scope: $scope" >&2
    usage
    exit 2
    ;;
esac

# Target only non-tracked, regenerable state.
targets=(
  ".cache"
  "tmp"
  "temp"
  "coverage"
  "dist"
  "build"
)

if [[ "$scope" == "python" || "$scope" == "all" ]]; then
  targets+=(".pytest_cache" ".mypy_cache" ".ruff_cache")
fi

if [[ "$scope" == "node" || "$scope" == "all" ]]; then
  targets+=("node_modules/.cache" ".next" ".nuxt" ".svelte-kit")
fi

echo "[reset-state] scope=$scope mode=$([[ "$apply" -eq 1 ]] && echo apply || echo dry-run)"

declare -a existing=()
for p in "${targets[@]}"; do
  if [[ -e "$p" ]]; then
    existing+=("$p")
  fi
done

if [[ ${#existing[@]} -eq 0 ]]; then
  echo "[reset-state] nothing to reset"
  exit 0
fi

echo "[reset-state] targets:"
for p in "${existing[@]}"; do
  echo "- $p"
done

if [[ "$apply" -eq 0 ]]; then
  echo "[reset-state] dry-run complete. Re-run with --apply to execute."
  exit 0
fi

for p in "${existing[@]}"; do
  rm -rf "$p"
done

echo "[reset-state] reset complete"
exit 0
