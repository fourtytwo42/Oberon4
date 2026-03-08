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
