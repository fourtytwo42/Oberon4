#!/usr/bin/env bash

failure_bundle_json_escape() {
  local value="${1-}"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  value="${value//$'\t'/\\t}"
  printf '%s' "${value}"
}

failure_bundle_create_dir() {
  local root_dir="$1"
  local entrypoint="$2"
  local bundle_root="${FAILURE_BUNDLE_ROOT:-${root_dir}/artifacts/failures}"
  local bundle_id
  bundle_id="$(date -u +%Y%m%dT%H%M%S)-${entrypoint}-$$"
  mkdir -p "${bundle_root}/${bundle_id}"
  printf '%s\n' "${bundle_root}/${bundle_id}"
}

failure_bundle_copy_file() {
  local source_path="$1"
  local destination_dir="$2"
  if [[ -f "${source_path}" ]]; then
    mkdir -p "${destination_dir}"
    cp "${source_path}" "${destination_dir}/$(basename "${source_path}")"
  fi
}

failure_bundle_write_metadata() {
  local output_path="$1"
  local entrypoint="$2"
  local failed_stage="$3"
  local primary_log_path="$4"
  cat >"${output_path}" <<EOF
{
  "timestamp_utc": "$(failure_bundle_json_escape "$(date -u +%Y-%m-%dT%H:%M:%SZ)")",
  "entrypoint": "$(failure_bundle_json_escape "${entrypoint}")",
  "status": "fail",
  "failed_stage": "$(failure_bundle_json_escape "${failed_stage}")",
  "primary_log_path": "$(failure_bundle_json_escape "${primary_log_path}")"
}
EOF
}

failure_bundle_capture_doctor() {
  local bundle_dir="$1"
  shift

  local doctor_output_file="${bundle_dir}/doctor-output.txt"
  local doctor_status_file="${bundle_dir}/doctor-status.txt"
  local doctor_json_file="${bundle_dir}/doctor.json"

  if "$@" >"${doctor_output_file}" 2>&1; then
    printf 'exit_code=0\n' >"${doctor_status_file}"
    cp "${doctor_output_file}" "${doctor_json_file}"
  else
    local exit_code=$?
    printf 'exit_code=%s\n' "${exit_code}" >"${doctor_status_file}"
    return 0
  fi
}

failure_bundle_capture_effective_config() {
  local bundle_dir="$1"
  shift

  local config_output_file="${bundle_dir}/effective-config.json"
  local config_status_file="${bundle_dir}/effective-config-status.txt"

  if "$@" >"${config_output_file}" 2>&1; then
    printf 'exit_code=0\n' >"${config_status_file}"
  else
    local exit_code=$?
    printf 'exit_code=%s\n' "${exit_code}" >"${config_status_file}"
    return 0
  fi
}
