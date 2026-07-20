#!/usr/bin/env bash
#
# install-user-service.sh
# Install the sample proton-syncd systemd user service and config.

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: install-user-service.sh [OPTIONS]

Options:
  --force-config   Replace an existing proton-sync.toml sample config
  --enable         Enable proton-syncd.service after installation
  --start          Start proton-syncd.service after installation
  --no-reload      Skip systemctl --user daemon-reload
  -h, --help       Show this help message
USAGE
}

err() {
  printf 'ERROR: %s\n' "$1" >&2
  exit 1
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    err "'${command_name}' is required but was not found"
  fi
}

parse_args() {
  force_config=false
  enable_service=false
  start_service=false
  reload_systemd=true

  while (($# > 0)); do
    case "$1" in
      --force-config)
        force_config=true
        ;;
      --enable)
        enable_service=true
        ;;
      --start)
        start_service=true
        ;;
      --no-reload)
        reload_systemd=false
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        err "unknown option: $1"
        ;;
    esac
    shift
  done
}

install_config() {
  local source_config="$1"
  local target_config="$2"
  local target_dir
  target_dir="$(dirname -- "${target_config}")"

  install -d -m 700 "${target_dir}"
  if [[ -e "${target_config}" && "${force_config}" != "true" ]]; then
    printf 'Keeping existing config: %s\n' "${target_config}"
    return
  fi
  install -m 600 "${source_config}" "${target_config}"
  printf 'Installed config: %s\n' "${target_config}"
}

install_service() {
  local source_service="$1"
  local target_service="$2"
  local target_dir
  target_dir="$(dirname -- "${target_service}")"

  install -d -m 755 "${target_dir}"
  install -m 644 "${source_service}" "${target_service}"
  printf 'Installed service: %s\n' "${target_service}"
}

reload_and_optionally_start() {
  if [[ "${reload_systemd}" == "true" ]]; then
    systemctl --user daemon-reload
  fi
  if [[ "${enable_service}" == "true" ]]; then
    systemctl --user enable proton-syncd.service
  fi
  if [[ "${start_service}" == "true" ]]; then
    systemctl --user start proton-syncd.service
  fi
}

main() {
  parse_args "$@"
  require_command install
  require_command systemctl

  local script_dir
  local repo_root
  local config_home
  local target_config
  local target_service
  script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd -- "${script_dir}/../.." && pwd)"
  config_home="${XDG_CONFIG_HOME:-${HOME}/.config}"
  target_config="${config_home}/proton-sync/proton-sync.toml"
  target_service="${config_home}/systemd/user/proton-syncd.service"

  install_config "${repo_root}/examples/proton-sync.toml" "${target_config}"
  install_service \
    "${repo_root}/examples/systemd/proton-syncd.service" \
    "${target_service}"
  reload_and_optionally_start

  printf 'Edit %s before starting the service with real data.\n' \
    "${target_config}"
}

main "$@"