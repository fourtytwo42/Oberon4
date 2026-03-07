#!/usr/bin/env bash
set -euo pipefail

load_hooks_env() {
  local env_file="${HOOKS_ENV_FILE:-scripts/hooks.env}"
  if [[ -f "$env_file" ]]; then
    # shellcheck disable=SC1090
    source "$env_file"
  fi
}

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

split_csv_to_space() {
  local csv="${1:-}"
  echo "$csv" | tr ',' ' ' | xargs
}

csv_contains() {
  local csv="${1:-}"
  local needle="${2:-}"
  local item
  for item in $(split_csv_to_space "$csv"); do
    if [[ "$item" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

load_stack_map() {
  local map_file="${STACK_MAP_FILE:-docs/policy/stack-map.env}"
  if [[ -f "$map_file" ]]; then
    # shellcheck disable=SC1090
    source "$map_file"
  fi
}

stack_regex_for_name() {
  local stack="$1"
  case "$stack" in
    node) printf '%s' "${STACK_NODE_PATH_REGEX:-(^|/)(package\.json|package-lock\.json|pnpm-lock\.yaml|yarn\.lock)|\.(ts|tsx|js|jsx)$}" ;;
    python) printf '%s' "${STACK_PYTHON_PATH_REGEX:-(^|/)(requirements(\.txt|-dev\.txt)?|pyproject\.toml|poetry\.lock)|\.py$}" ;;
    go) printf '%s' "${STACK_GO_PATH_REGEX:-(^|/)(go\.mod|go\.sum)|\.go$}" ;;
    java) printf '%s' "${STACK_JAVA_PATH_REGEX:-(^|/)(pom\.xml|build\.gradle(\.kts)?|gradle\.lockfile)|\.java$}" ;;
    dotnet) printf '%s' "${STACK_DOTNET_PATH_REGEX:-\.(cs|csproj|sln)$}" ;;
    rust) printf '%s' "${STACK_RUST_PATH_REGEX:-(^|/)(Cargo\.toml|Cargo\.lock)|\.rs$}" ;;
    php) printf '%s' "${STACK_PHP_PATH_REGEX:-(^|/)(composer\.json|composer\.lock)|\.php$}" ;;
    ruby) printf '%s' "${STACK_RUBY_PATH_REGEX:-(^|/)(Gemfile|Gemfile\.lock)|\.rb$}" ;;
    *) printf '%s' "" ;;
  esac
}

detect_stacks_from_paths() {
  local changed_paths="${1:-}"
  local active="${STACK_ACTIVE_PROFILES:-node,python,go,java,dotnet,rust,php,ruby}"
  local found=""
  local stack regex

  for stack in $(split_csv_to_space "$active"); do
    regex="$(stack_regex_for_name "$stack")"
    [[ -z "$regex" ]] && continue
    if grep -Eiq "$regex" <<<"$changed_paths"; then
      if [[ -z "$found" ]]; then
        found="$stack"
      else
        found="${found},${stack}"
      fi
    fi
  done

  printf '%s\n' "$found"
}

repo_slug_from_remote() {
  local remote
  remote="$(git remote get-url origin 2>/dev/null || true)"
  if [[ -z "$remote" ]]; then
    return 0
  fi

  # Matches:
  # - git@github.com:owner/repo.git
  # - https://github.com/owner/repo.git
  # - https://github.com/owner/repo
  if [[ "$remote" =~ github\.com[:/]([^/]+)/([^/.]+)(\.git)?$ ]]; then
    printf '%s/%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
    return 0
  fi
}

resolve_repo_slug() {
  if [[ -n "${HOOKS_REPO_SLUG:-}" ]]; then
    printf '%s\n' "$HOOKS_REPO_SLUG"
    return 0
  fi
  if [[ -n "${GITHUB_REPOSITORY:-}" ]]; then
    printf '%s\n' "$GITHUB_REPOSITORY"
    return 0
  fi
  repo_slug_from_remote || true
}

issue_url_regex_for_repo() {
  local slug="$1"
  local strict_same_repo="${HOOKS_REQUIRE_SAME_REPO_URL:-1}"

  if [[ "$strict_same_repo" == "1" && -n "$slug" ]]; then
    local esc_slug
    esc_slug="${slug//\//\\/}"
    printf 'https://github\\.com/%s/issues/[0-9]+' "$esc_slug"
    return 0
  fi

  printf 'https://github\\.com/[^/]+/[^/]+/issues/[0-9]+'
}
