#!/usr/bin/env bash

runtime_default_data_dir() {
  if [[ -n "${OBERON4_DATA_DIR:-}" ]]; then
    printf '%s\n' "${OBERON4_DATA_DIR}"
    return
  fi
  printf '%s/.oberon4\n' "${HOME:-.}"
}

runtime_default_socket_path() {
  if [[ -n "${OBERON4_SOCKET_PATH:-}" ]]; then
    printf '%s\n' "${OBERON4_SOCKET_PATH}"
    return
  fi
  printf '%s/oberon4-daemon.sock\n' "${TMPDIR:-/tmp}"
}

runtime_materialize_secret_env() {
  local env_name="$1"
  local data_dir="${2:-$(runtime_default_data_dir)}"

  if [[ -z "${env_name}" ]]; then
    return 0
  fi
  if [[ -n "${!env_name:-}" ]]; then
    return 0
  fi

  local secrets_path="${data_dir}/secrets.json"
  if [[ ! -f "${secrets_path}" ]]; then
    return 0
  fi

  local value
  value="$(ENV_NAME="${env_name}" SECRETS_PATH="${secrets_path}" node <<'NODE'
const fs = require("fs");

const envName = process.env.ENV_NAME;
const secretsPath = process.env.SECRETS_PATH;

try {
  const raw = fs.readFileSync(secretsPath, "utf8");
  const decoded = JSON.parse(raw);
  const value = decoded[`env.${envName}`];
  if (typeof value === "string") {
    process.stdout.write(value);
  }
} catch (_err) {
  process.exit(0);
}
NODE
)"
  if [[ -n "${value}" ]]; then
    export "${env_name}=${value}"
  fi
}
