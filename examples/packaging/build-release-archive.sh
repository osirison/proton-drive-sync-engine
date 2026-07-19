#!/usr/bin/env bash
#
# build-release-archive.sh
# Build release binaries and package the user-service distribution assets.

set -euo pipefail

release_staging_dir=""

usage() {
  cat <<'USAGE'
Usage: build-release-archive.sh [OPTIONS]

Options:
  --archive-path PATH  Write the archive to PATH
  --skip-build         Package existing release binaries without running cargo
  -h, --help           Show this help message
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
  archive_path=""
  skip_build=false

  while (($# > 0)); do
    case "$1" in
      --archive-path)
        if [[ -z "${2:-}" || "$2" == --* ]]; then
          err '--archive-path requires a path'
        fi
        archive_path="$2"
        shift 2
        ;;
      --skip-build)
        skip_build=true
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        err "unknown option: $1"
        ;;
    esac
  done
}

resolve_target_dir() {
  local repo_root="$1"
  local configured_target_dir="${CARGO_TARGET_DIR:-target}"

  if [[ "${configured_target_dir}" = /* ]]; then
    printf '%s\n' "${configured_target_dir}"
  else
    printf '%s\n' "${repo_root}/${configured_target_dir}"
  fi
}

package_version() {
  local repo_root="$1"
  awk -F '"' '/^version = / { print $2; exit }' "${repo_root}/Cargo.toml"
}

install_release_asset() {
  local source_path="$1"
  local target_path="$2"
  local mode="$3"

  if [[ ! -e "${source_path}" ]]; then
    err "missing release asset: ${source_path}"
  fi
  install -D -m "${mode}" "${source_path}" "${target_path}"
}

build_release_binaries() {
  local repo_root="$1"

  if [[ "${skip_build}" == "true" ]]; then
    return
  fi
  cargo build --manifest-path "${repo_root}/Cargo.toml" --release --bins --locked
}

create_archive() {
  local repo_root="$1"
  local target_dir="$2"
  local version="$3"
  local archive_path="$4"
  local staging_dir="$5"
  local package_root
  package_root="${staging_dir}/proton-drive-sync-engine-${version}"

  install_release_asset \
    "${target_dir}/release/proton-syncd" \
    "${package_root}/bin/proton-syncd" \
    755
  install_release_asset \
    "${target_dir}/release/proton-sync" \
    "${package_root}/bin/proton-sync" \
    755
  install_release_asset \
    "${repo_root}/README.md" \
    "${package_root}/README.md" \
    644
  install_release_asset \
    "${repo_root}/LICENSE" \
    "${package_root}/LICENSE" \
    644
  install_release_asset \
    "${repo_root}/examples/proton-sync.toml" \
    "${package_root}/examples/proton-sync.toml" \
    600
  install_release_asset \
    "${repo_root}/examples/systemd/proton-syncd.service" \
    "${package_root}/examples/systemd/proton-syncd.service" \
    644
  install_release_asset \
    "${repo_root}/examples/systemd/install-user-service.sh" \
    "${package_root}/examples/systemd/install-user-service.sh" \
    755
  install_release_asset \
    "${repo_root}/examples/packaging/release-assets.toml" \
    "${package_root}/examples/packaging/release-assets.toml" \
    644

  mkdir -p "$(dirname -- "${archive_path}")"
  tar -C "${staging_dir}" -czf "${archive_path}" "$(basename -- "${package_root}")"
}

cleanup() {
  if [[ -n "${release_staging_dir}" ]]; then
    rm -rf -- "${release_staging_dir}"
  fi
}

main() {
  parse_args "$@"
  require_command awk
  require_command cargo
  require_command install
  require_command mktemp
  require_command tar

  local script_dir
  local repo_root
  local target_dir
  local version
  script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd -- "${script_dir}/../.." && pwd)"
  target_dir="$(resolve_target_dir "${repo_root}")"
  version="$(package_version "${repo_root}")"
  if [[ -z "${version}" ]]; then
    err 'Cargo.toml package version was not found'
  fi
  if [[ -z "${archive_path}" ]]; then
    archive_path="${repo_root}/target/dist/proton-drive-sync-engine-${version}.tar.gz"
  fi

  release_staging_dir="$(mktemp -d)"
  trap cleanup EXIT

  build_release_binaries "${repo_root}"
  create_archive "${repo_root}" "${target_dir}" "${version}" \
    "${archive_path}" "${release_staging_dir}"

  printf 'Created release archive: %s\n' "${archive_path}"
}

main "$@"