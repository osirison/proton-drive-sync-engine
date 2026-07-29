#!/usr/bin/env bash
#
# upgrade.sh — upgrade an existing Proton Drive Sync install in place.
#
# Rebuilds and reinstalls the binaries from this checkout, refreshes the systemd user unit and (if the
# desktop app is installed) its launcher so template changes ship to existing installs, then restarts
# the running service. Your config, folder pairing, and sync state are left untouched.
#
#   ./upgrade.sh                 # rebuild everything that is installed, refresh unit/launcher, restart
#   ./upgrade.sh --pull          # git pull --ff-only first, then upgrade
#   ./upgrade.sh --engine-only   # skip the desktop app even if it is installed
#   ./upgrade.sh --no-build      # only refresh the unit/launcher (no rebuild), then restart
#
# For a first-time install use ./setup.sh. To remove everything use ./uninstall.sh.
#
# This reuses setup.sh's installer functions (install_unit, install_desktop_launcher, resolve_*_bin,
# ensure_gui_build_deps, cargo_bin_dir, and the config/xdg/output helpers) by sourcing it, so the unit
# and launcher it writes are byte-for-byte what a fresh setup.sh would write — no template drift.

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=setup.sh
source "${script_dir}/setup.sh" # defines err/warn/step/note + the installer functions; main is guarded
repo_root="${script_dir}"       # consumed by install_engine_binaries / install_desktop_launcher

# ---- usage ------------------------------------------------------------------------------------

usage() {
  cat <<'USAGE'
Usage:
  upgrade.sh [OPTIONS]        # rebuild + refresh an existing install, then restart the service

Options:
  --pull                 git pull --ff-only in this checkout before building (skipped if not a clean
                         fast-forward; upgrade continues with whatever is checked out).
  --gui                  Force upgrading the desktop app even if it is not currently detected.
  --engine-only          Skip the desktop app (upgrade only proton-syncd / proton-sync).
  --no-build             Skip the rebuild; only refresh the unit/launcher and restart. Assumes the
                         binaries already on PATH are the new ones.
  --no-restart           Do not restart the service (leave the old process running until you restart).
  --install-deps         With a GUI upgrade, install the desktop app's system build deps with sudo.
  --config PATH          Config the service points at. Default: read from the installed unit, else the
                         XDG default. Only used to point the refreshed unit at the same file.
  -h, --help             Show this help.
USAGE
}

# ---- argument parsing -------------------------------------------------------------------------

parse_upgrade_args() {
  do_pull=false
  force_gui=false
  engine_only=false
  no_build=false     # read by setup.sh's install_*_binaries / resolve_installed_bin
  no_restart=false
  install_deps=false # read by setup.sh's ensure_gui_build_deps
  config_path=""

  while (($# > 0)); do
    case "$1" in
      --pull) do_pull=true ;;
      --gui) force_gui=true ;;
      --engine-only) engine_only=true ;;
      --no-build) no_build=true ;;
      --no-restart) no_restart=true ;;
      --install-deps) install_deps=true ;;
      --config | --config=*) take_value "$1" "${2-}" && shift; config_path="${REPLY}" ;;
      -h | --help) usage; exit 0 ;;
      *) err "unknown option: $1 (try --help)" ;;
    esac
    shift
  done

  if [[ "${force_gui}" == "true" && "${engine_only}" == "true" ]]; then
    err "--gui and --engine-only are mutually exclusive"
  fi
}

# ---- install detection ------------------------------------------------------------------------

unit_path() {
  printf '%s/systemd/user/proton-syncd.service\n' "$(config_home)"
}

desktop_launcher_path() {
  printf '%s/applications/app.protondrivesync.engine.desktop\n' "$(xdg_data_home)"
}

# Pull the --config value out of the installed unit's ExecStart so the refreshed unit keeps pointing
# at the same file (it may not be the XDG default). Empty if the unit is absent or has no --config.
config_path_from_unit() {
  local unit
  unit="$(unit_path)"
  [[ -f "${unit}" ]] || return 0
  # ExecStart="<bin>" --config "<cfg>"  — prefer the quoted form, fall back to a bare token.
  # Best-effort: a unit with no matching ExecStart makes grep exit non-zero, which under
  # `set -e` would abort the whole upgrade via the `detected_config="$(...)"` substitution. The
  # trailing `|| true` yields empty output instead, and the caller falls back to the XDG default.
  grep -E '^ExecStart=' "${unit}" 2>/dev/null | head -n1 | sed -nE \
    -e 's/.*--config[[:space:]]+"([^"]+)".*/\1/p;t' \
    -e 's/.*--config[[:space:]]+([^[:space:]]+).*/\1/p' \
    | head -n1 || true
}

gui_is_installed() {
  [[ -f "$(desktop_launcher_path)" ]] && return 0
  command -v proton-sync-gui >/dev/null 2>&1
}

# ---- git ---------------------------------------------------------------------------------------

pull_latest() {
  [[ "${do_pull}" == "true" ]] || return 0
  step "Fetching the latest source (git pull --ff-only)"
  if ! command -v git >/dev/null 2>&1; then
    warn "git not found — skipping --pull; building the currently checked-out source"
    return 0
  fi
  if ! git -C "${repo_root}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    warn "not a git checkout — skipping --pull; building the current source"
    return 0
  fi
  if git -C "${repo_root}" pull --ff-only; then
    note "Updated to $(git -C "${repo_root}" rev-parse --short HEAD)"
  else
    warn "git pull --ff-only did not fast-forward (local commits, detached HEAD, or offline)."
    warn "Continuing with the currently checked-out source."
  fi
}

# ---- main --------------------------------------------------------------------------------------

main() {
  parse_upgrade_args "$@"

  [[ -f "${repo_root}/Cargo.toml" ]] || err "run upgrade.sh from the proton-drive-sync-engine checkout (Cargo.toml not found at ${repo_root})"

  # Refuse to "upgrade" a machine that has nothing installed — that is a fresh install, not an upgrade.
  # `if` blocks, not `[[ ]] && var=true`: a false test returns non-zero and under `set -e` would abort.
  local have_unit=false have_config=false have_binaries=false
  if [[ -f "$(unit_path)" ]]; then have_unit=true; fi
  local detected_config
  detected_config="$(config_path_from_unit)"
  if [[ -n "${config_path}" && -f "${config_path}" ]]; then have_config=true; fi
  if [[ -n "${detected_config}" && -f "${detected_config}" ]]; then have_config=true; fi
  if [[ -f "$(default_config_path)" ]]; then have_config=true; fi
  if command -v proton-syncd >/dev/null 2>&1; then have_binaries=true; fi
  if [[ "${have_unit}" == "false" && "${have_config}" == "false" && "${have_binaries}" == "false" ]]; then
    err "no existing install found (no systemd unit, config, or proton-syncd on PATH). Run ./setup.sh for a first install."
  fi

  # Resolve the config path the refreshed unit should point at: explicit flag wins, then the value
  # already baked into the installed unit, then the XDG default.
  if [[ -z "${config_path}" ]]; then
    config_path="${detected_config:-$(default_config_path)}"
  fi

  # Decide GUI scope up front.
  local upgrade_gui=false
  if [[ "${force_gui}" == "true" ]]; then
    upgrade_gui=true
  elif [[ "${engine_only}" == "true" ]]; then
    upgrade_gui=false
  elif gui_is_installed; then
    upgrade_gui=true
  fi

  step "Upgrading Proton Drive Sync"
  note "Checkout:   ${repo_root}"
  note "Config:     ${config_path}"
  note "Desktop app: $([[ "${upgrade_gui}" == "true" ]] && echo "yes" || echo "no")"
  note "Your config, folder pairing, and sync state are preserved."

  pull_latest

  # Remember whether the service is currently running so we know whether to bring it back up.
  local was_active=false
  if systemctl --user is-active --quiet proton-syncd.service 2>/dev/null; then
    was_active=true
  fi

  # --- rebuild binaries -------------------------------------------------------------------------
  install_engine_binaries
  local daemon_bin
  daemon_bin="$(resolve_daemon_bin)"

  local gui_bin=""
  local gui_ok=true
  if [[ "${upgrade_gui}" == "true" ]]; then
    # A missing GUI build dep must not abort the whole upgrade — the engine is already rebuilt. Run the
    # GUI build in a subshell so its err/exit is caught here, then warn and finish the engine upgrade.
    if (install_gui_binary); then
      gui_bin="$(resolve_gui_bin)"
    else
      gui_ok=false
      warn "the desktop app did not rebuild (see above) — the engine was still upgraded."
      warn "install the build deps (or pass --install-deps) and re-run to refresh the desktop app."
    fi
  fi

  # --- refresh unit + launcher (ship template changes) ------------------------------------------
  install_unit "${daemon_bin}" "${config_path}"

  if [[ "${upgrade_gui}" == "true" && "${gui_ok}" == "true" && -n "${gui_bin}" ]]; then
    install_desktop_launcher "${gui_bin}" "$(dirname -- "${daemon_bin}")"
  fi

  # --- restart --------------------------------------------------------------------------------
  if [[ "${no_restart}" == "true" ]]; then
    step "Not restarting the service (--no-restart)"
    note "The new binary takes effect on the next restart:  systemctl --user restart proton-syncd"
  elif [[ "${was_active}" == "true" ]]; then
    step "Restarting proton-syncd to pick up the new binary"
    systemctl --user restart proton-syncd.service
    note "Service restarted. Verify with:"
    note "  proton-sync status"
    note "  systemctl --user status proton-syncd"
  else
    step "Service is not running — leaving it stopped"
    note "Start it when ready with:  systemctl --user start proton-syncd"
  fi

  step "Upgrade complete"
}

main "$@"
