#!/usr/bin/env bash
#
# uninstall.sh — remove Proton Drive Sync from this machine, leaving it clean.
#
# Stops and removes the systemd user service, uninstalls the binaries, and deletes the config, the
# desktop app's files, and all engine state (both the current per-root `<local_root>/.sync` layout and
# the older `$XDG_STATE_HOME/proton-drive-sync` one). It NEVER touches the files you were syncing
# (only the `.sync` state dir inside your local root), nor the separate `proton-drive` CLI or its
# login — those are yours, not ours.
#
#   ./uninstall.sh --dry-run     # show exactly what would be removed, change nothing
#   ./uninstall.sh               # remove everything (prompts once before deleting)
#   ./uninstall.sh -y            # remove everything without prompting
#   ./uninstall.sh --keep-config # remove everything but keep the config + folder pairing
#
# Reuses setup.sh's config/xdg/output helpers by sourcing it (its main is guarded, so sourcing only
# defines functions).

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=setup.sh
source "${script_dir}/setup.sh" # config_home/default_config_path/config_value/expand_tilde/xdg_data_home/cargo_bin_dir + err/warn/step/note

# ---- usage ------------------------------------------------------------------------------------

usage() {
  cat <<'USAGE'
Usage:
  uninstall.sh [OPTIONS]     # remove Proton Drive Sync from this machine

Options:
  --dry-run              Print the removal plan and exit; change nothing.
  -y, --yes              Do not prompt for confirmation before removing.
  --keep-config          Keep the config file + folder pairing (remove everything else).
  --config PATH          Config to read the local root from (to find its .sync state dir).
                         Default: $XDG_CONFIG_HOME/proton-sync/proton-sync.toml.
  -h, --help             Show this help.

Always preserved: the files you were syncing (your local folder's contents), the proton-drive CLI,
and its login/session. Per-directory .proton-sync.toml files inside your synced tree are also left.
USAGE
}

# ---- argument parsing -------------------------------------------------------------------------

parse_uninstall_args() {
  dry_run=false
  assume_yes=false
  keep_config=false
  config_path=""

  while (($# > 0)); do
    case "$1" in
      --dry-run) dry_run=true ;;
      -y | --yes) assume_yes=true ;;
      --keep-config) keep_config=true ;;
      --config | --config=*) take_value "$1" "${2-}" && shift; config_path="${REPLY}" ;;
      -h | --help) usage; exit 0 ;;
      *) err "unknown option: $1 (try --help)" ;;
    esac
    shift
  done
}

# ---- path helpers -----------------------------------------------------------------------------

xdg_cache_home() { printf '%s\n' "${XDG_CACHE_HOME:-${HOME}/.cache}"; }
user_state_home() { printf '%s\n' "${XDG_STATE_HOME:-${HOME}/.local/state}"; }

unit_path() { printf '%s/systemd/user/proton-syncd.service\n' "$(config_home)"; }
desktop_launcher_path() { printf '%s/applications/app.protondrivesync.engine.desktop\n' "$(xdg_data_home)"; }
desktop_icon_path() { printf '%s/icons/hicolor/scalable/apps/app.protondrivesync.engine.svg\n' "$(xdg_data_home)"; }
runtime_socket_path() { printf '%s/proton-sync.sock\n' "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}/proton-drive-sync-$(id -u)}"; }
fallback_runtime_dir() { printf '%s/proton-drive-sync-%s\n' "${TMPDIR:-/tmp}" "$(id -u)"; }

# The engine's per-root state dir, resolved and validated so we can never `rm -rf` anything but a real
# `.sync` directory. local_root comes from a possibly hand-edited config, so it is not trusted: it must
# resolve to an existing directory that is not `/` or $HOME, and the child we delete must be named
# `.sync`. Prints the validated `<root>/.sync` path, or nothing (with return 1) if it is unsafe/absent.
validated_sync_state_dir() {
  local root="$1"
  [[ -n "${root}" ]] || return 1
  root="$(expand_tilde "${root}")"
  [[ -d "${root}" ]] || return 1
  root="$(cd -- "${root}" && pwd)" || return 1
  case "${root}" in
    "/" | "${HOME}") return 1 ;;
  esac
  local sync="${root%/}/.sync"
  [[ "$(basename -- "${sync}")" == ".sync" ]] || return 1
  [[ -d "${sync}" ]] || return 1
  printf '%s\n' "${sync}"
}

# ---- removal primitive ------------------------------------------------------------------------

# Delete a file or directory, honoring --dry-run, and never abort the run on a single failure (a
# half-uninstall is worse than a warning). No-op if the path does not exist.
remove_path() {
  local path="$1" desc="$2"
  [[ -e "${path}" || -L "${path}" ]] || return 0
  if [[ "${dry_run}" == "true" ]]; then
    note "would remove ${desc}: ${path}"
    return 0
  fi
  if rm -rf -- "${path}"; then
    note "removed ${desc}: ${path}"
  else
    warn "could not remove ${desc}: ${path} (remove it by hand)"
  fi
}

# ---- plan (existence checks only; no side effects) --------------------------------------------

# Build the list of state/asset paths that currently exist. Kept in one place so the plan preview and
# the apply step operate on exactly the same set.
collect_targets() {
  removable_paths=() # "path\tdescription"
  add() {
    local path="$1" desc="$2"
    # Use an if (not `[[ ]] && ...`): a false test makes the compound return non-zero, which under
    # `set -e` would abort the whole uninstall the first time a path happens not to exist.
    if [[ -e "${path}" || -L "${path}" ]]; then
      removable_paths+=("${path}"$'\t'"${desc}")
    fi
  }

  if [[ "${keep_config}" != "true" ]]; then
    # Only remove the whole *directory* when it is the dedicated one setup.sh creates
    # ($XDG_CONFIG_HOME/proton-sync). For a custom --config path the parent is a shared directory
    # (e.g. ~/.config), so `rm -rf`ing it would be catastrophic — delete just the config file there.
    local config_default_dir config_dir
    config_default_dir="$(config_home)/proton-sync"
    config_dir="$(dirname -- "${config_path}")"
    if [[ "${config_dir}" == "${config_default_dir}" ]]; then
      add "${config_default_dir}" "config directory"
    else
      add "${config_path}" "config file"
    fi
  fi
  add "$(desktop_launcher_path)" "desktop launcher"
  add "$(desktop_icon_path)" "desktop icon"
  add "$(xdg_data_home)/app.protondrivesync.engine" "desktop app data"
  add "$(config_home)/app.protondrivesync.engine" "desktop app config"
  add "$(xdg_cache_home)/app.protondrivesync.engine" "desktop app cache"
  add "$(user_state_home)/proton-drive-sync" "global state (lock + legacy index)"
  add "$(runtime_socket_path)" "control socket"
  add "$(fallback_runtime_dir)" "fallback runtime directory"

  # Per-root .sync — only if it validates as a real `.sync` under the configured local root.
  local sync_dir
  if sync_dir="$(validated_sync_state_dir "${local_root}")"; then
    removable_paths+=("${sync_dir}"$'\t'"engine state (per-root .sync)")
  fi
}

service_installed() {
  [[ -f "$(unit_path)" ]] && return 0
  systemctl --user cat proton-syncd.service >/dev/null 2>&1
}

# Which cargo packages are actually cargo-installed (so we uninstall only what setup.sh installed, and
# never guess-delete a binary that arrived some other way, e.g. a --no-build PATH install).
cargo_installed_packages() {
  command -v cargo >/dev/null 2>&1 || return 0
  local pkg
  for pkg in proton-drive-sync-engine proton-sync-gui; do
    if cargo install --list 2>/dev/null | grep -q "^${pkg} v"; then
      printf '%s\n' "${pkg}"
    fi
  done
}

print_plan() {
  step "Uninstall plan"

  if service_installed; then
    note "systemd service:  stop, disable, and remove the proton-syncd user unit"
  else
    note "systemd service:  (not installed)"
  fi

  local pkgs
  pkgs="$(cargo_installed_packages)"
  if [[ -n "${pkgs}" ]]; then
    note "binaries:         cargo uninstall $(echo "${pkgs}" | paste -sd' ' -)"
  else
    note "binaries:         (none cargo-installed)"
    # A --no-build install leaves binaries on PATH that cargo does not track — surface them, but do
    # not delete a binary we did not install.
    local name found=""
    for name in proton-syncd proton-sync proton-sync-gui; do
      local path
      path="$(command -v "${name}" 2>/dev/null || true)"
      if [[ -n "${path}" ]]; then
        found+="    ${name} -> ${path}"$'\n'
      fi
    done
    if [[ -n "${found}" ]]; then
      warn "these binaries are on PATH but were not cargo-installed — remove them by hand if you want them gone:"
      printf '%s' "${found}" >&2
    fi
  fi

  if ((${#removable_paths[@]} == 0)); then
    note "files/state:      (nothing found to remove)"
  else
    note "files/state to remove:"
    local entry path desc
    for entry in "${removable_paths[@]}"; do
      path="${entry%%$'\t'*}"
      desc="${entry##*$'\t'}"
      note "    ${desc}: ${path}"
    done
  fi

  step "Preserved (never touched)"
  note "your synced files:        ${local_root:-<local root from config>} (only its .sync state dir is removed)"
  if [[ "${keep_config}" == "true" ]]; then
    note "config + folder pairing:  ${config_path} (--keep-config)"
  fi
  note "the proton-drive CLI and its login/session"
  note "per-directory .proton-sync.toml files inside your synced tree"
}

# ---- apply ------------------------------------------------------------------------------------

remove_service() {
  service_installed || return 0
  if [[ "${dry_run}" == "true" ]]; then
    note "would stop + disable + remove the proton-syncd systemd user unit"
    return 0
  fi
  step "Stopping and removing the systemd service"
  # disable --now stops it too; SIGTERM is the daemon's graceful-shutdown path. Best-effort throughout.
  systemctl --user disable --now proton-syncd.service >/dev/null 2>&1 || true
  systemctl --user reset-failed proton-syncd.service >/dev/null 2>&1 || true
  remove_path "$(unit_path)" "systemd user unit"
  systemctl --user daemon-reload >/dev/null 2>&1 || true
  note "Removed and reloaded the systemd user manager"
}

remove_binaries() {
  local pkgs pkg
  pkgs="$(cargo_installed_packages)"
  [[ -n "${pkgs}" ]] || return 0
  if [[ "${dry_run}" == "true" ]]; then
    note "would run: cargo uninstall $(echo "${pkgs}" | paste -sd' ' -)"
    return 0
  fi
  step "Uninstalling the binaries"
  while IFS= read -r pkg; do
    [[ -n "${pkg}" ]] || continue
    if cargo uninstall "${pkg}" >/dev/null 2>&1; then
      note "cargo uninstall ${pkg}"
    else
      warn "cargo uninstall ${pkg} failed — remove its binaries from $(cargo_bin_dir) by hand"
    fi
  done <<<"${pkgs}"
}

remove_files() {
  ((${#removable_paths[@]} > 0)) || return 0
  [[ "${dry_run}" == "true" ]] || step "Removing config, desktop files, and engine state"
  local entry path desc
  for entry in "${removable_paths[@]}"; do
    path="${entry%%$'\t'*}"
    desc="${entry##*$'\t'}"
    remove_path "${path}" "${desc}"
  done
}

refresh_desktop_caches() {
  [[ "${dry_run}" == "true" ]] && return 0
  command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$(xdg_data_home)/applications" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -f -t "$(xdg_data_home)/icons/hicolor" >/dev/null 2>&1 || true
}

# ---- main --------------------------------------------------------------------------------------

main() {
  parse_uninstall_args "$@"

  if [[ -z "${config_path}" ]]; then
    config_path="$(default_config_path)"
  fi

  # Read the local root from the config so we can find (and only then, safely, remove) its .sync dir.
  local_root=""
  if [[ -f "${config_path}" ]]; then
    local_root="$(config_value "${config_path}" local_root)"
  fi

  collect_targets
  print_plan

  if [[ "${dry_run}" == "true" ]]; then
    step "Dry run — nothing was changed"
    return 0
  fi

  if [[ "${assume_yes}" != "true" ]]; then
    if [[ -t 0 ]]; then
      local reply
      printf '\n'
      read -r -p "Remove Proton Drive Sync as shown above? [y/N] " reply
      if [[ ! "${reply}" =~ ^[Yy]$ ]]; then
        step "Aborted — nothing was changed"
        return 0
      fi
    else
      err "refusing to uninstall non-interactively without -y/--yes (re-run with --dry-run to preview, or -y to proceed)"
    fi
  fi

  remove_service
  remove_binaries
  remove_files
  refresh_desktop_caches

  step "Done — Proton Drive Sync has been removed"
  note "Your synced files were left in place. If you are finished with Proton Drive entirely, log out"
  note "of and remove the separate 'proton-drive' CLI yourself."
}

main "$@"
