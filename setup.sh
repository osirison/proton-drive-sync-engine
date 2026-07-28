#!/usr/bin/env bash
#
# setup.sh — one-command install for Proton Drive Sync.
#
# Installs the engine binaries, writes a config the systemd user service reads, and gets you
# syncing. Two modes:
#
#   ./setup.sh --local-root ~/ProtonDrive --remote-root /Drive/RemoteFolder
#       Terminal quick setup (default): build + install the daemon and CLI, generate the config,
#       install & reload the systemd user unit, preview a dry-run, then enable + start the service.
#
#   ./setup.sh --gui
#       Non-terminal setup: also build + install the desktop app, install its launcher, then open
#       it straight into the onboarding wizard, which picks the folders (persisting them to the same
#       config the service reads), reviews a dry-run, and starts the service. No further terminal use.
#
# The generated config lives at $XDG_CONFIG_HOME/proton-sync/proton-sync.toml (default
# ~/.config/proton-sync/proton-sync.toml) and the unit runs `proton-syncd --config <that file>`.

set -euo pipefail

# ---- output helpers ---------------------------------------------------------------------------

err() {
  printf 'ERROR: %s\n' "$1" >&2
  exit 1
}
warn() {
  printf 'WARNING: %s\n' "$1" >&2
}
step() {
  printf '\n==> %s\n' "$1"
}
note() {
  printf '    %s\n' "$1"
}

require_command() {
  local command_name="$1"
  local hint="${2:-}"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    if [[ -n "${hint}" ]]; then
      err "'${command_name}' is required but was not found — ${hint}"
    fi
    err "'${command_name}' is required but was not found"
  fi
}

# ---- usage ------------------------------------------------------------------------------------

usage() {
  cat <<'USAGE'
Usage:
  setup.sh --local-root DIR --remote-root PATH [OPTIONS]   # terminal quick setup (default)
  setup.sh --gui [OPTIONS]                                 # install + launch GUI onboarding

Terminal quick setup options:
  --local-root DIR       Local folder to sync (created if missing). Prompted if omitted on a TTY.
  --remote-root PATH     Proton Drive path to sync it to (e.g. /Drive/RemoteFolder).
  --config PATH          Config file to write/point the service at
                         (default: $XDG_CONFIG_HOME/proton-sync/proton-sync.toml).
  --proton-cli PATH      Path to the proton-drive CLI if it is not on PATH.
  --force-config         Overwrite an existing config file (default: keep it).
  --no-start             Install everything and preview a dry-run, but do not enable/start the
                         service (leaves it ready for `systemctl --user enable --now proton-syncd`).
  -y, --yes              Non-interactive: don't prompt; assume yes at the start confirmation.

GUI setup:
  --gui                  Build + install the desktop app and its launcher, then open the onboarding
                         wizard. The wizard chooses the folders and starts the service — the folder
                         flags above are ignored in this mode.
  --install-deps         Attempt to install the GUI's system build dependencies with sudo.

Common options:
  --no-build             Skip `cargo install`; assume the binaries are already on PATH.
  -h, --help             Show this help.
USAGE
}

# ---- argument parsing -------------------------------------------------------------------------

# Split `--key=value` into the value; otherwise consume the next argument. Sets REPLY.
take_value() {
  local current="$1"
  local next="${2-}"
  if [[ "${current}" == *=* ]]; then
    REPLY="${current#*=}"
    return 1 # consumed only the current token
  fi
  if [[ -z "${next}" || "${next}" == -* ]]; then
    err "option '${current}' requires a value"
  fi
  REPLY="${next}"
  return 0 # consumed the next token too
}

parse_args() {
  gui_mode=false
  local_root=""
  remote_root=""
  config_path=""
  proton_cli=""
  force_config=false
  no_start=false
  assume_yes=false
  no_build=false
  install_deps=false

  while (($# > 0)); do
    case "$1" in
      --gui) gui_mode=true ;;
      --local-root|--local-root=*) take_value "$1" "${2-}" && shift; local_root="${REPLY}" ;;
      --remote-root|--remote-root=*) take_value "$1" "${2-}" && shift; remote_root="${REPLY}" ;;
      --config|--config=*) take_value "$1" "${2-}" && shift; config_path="${REPLY}" ;;
      --proton-cli|--proton-cli=*) take_value "$1" "${2-}" && shift; proton_cli="${REPLY}" ;;
      --force-config) force_config=true ;;
      --no-start) no_start=true ;;
      -y|--yes) assume_yes=true ;;
      --no-build) no_build=true ;;
      --install-deps) install_deps=true ;;
      -h|--help) usage; exit 0 ;;
      *) err "unknown option: $1 (try --help)" ;;
    esac
    shift
  done
}

# ---- path helpers -----------------------------------------------------------------------------

expand_tilde() {
  # Expand a leading ~/ that survived because the value was quoted.
  local value="$1"
  if [[ "${value}" == "~/"* ]]; then
    printf '%s\n' "${HOME}/${value:2}"
  else
    printf '%s\n' "${value}"
  fi
}

config_home() {
  printf '%s\n' "${XDG_CONFIG_HOME:-${HOME}/.config}"
}

default_config_path() {
  printf '%s/proton-sync/proton-sync.toml\n' "$(config_home)"
}

# Read a top-level string value out of a TOML file (best-effort; empty if absent). Used only to warn
# about a folder-pair mismatch on re-run, not to parse config for real.
config_value() {
  local file="$1" key="$2"
  grep -E "^[[:space:]]*${key}[[:space:]]*=" "${file}" 2>/dev/null \
    | head -n1 \
    | sed -E "s/^[[:space:]]*${key}[[:space:]]*=[[:space:]]*\"?([^\"#]*[^\"# ])\"?.*/\1/" || true
}

# Where `cargo install` drops binaries. Cargo's own precedence is CARGO_INSTALL_ROOT > CARGO_HOME >
# ~/.cargo (a `[install] root` in cargo config also wins, but that we can't read here — resolve_*_bin
# falls back to `command -v` for that case).
cargo_bin_dir() {
  if [[ -n "${CARGO_INSTALL_ROOT:-}" ]]; then
    printf '%s/bin\n' "${CARGO_INSTALL_ROOT}"
  else
    printf '%s/bin\n' "${CARGO_HOME:-${HOME}/.cargo}"
  fi
}

# ---- prerequisite checks ----------------------------------------------------------------------

check_common_prereqs() {
  require_command systemctl "this installs a systemd *user* service; systemd is required"
  require_command install
  if [[ "${no_build}" != "true" ]]; then
    require_command cargo "install the Rust toolchain (https://rustup.rs), edition 2024 / Rust >= 1.85"
  fi

  local cli="${proton_cli:-proton-drive}"
  if ! command -v "${cli}" >/dev/null 2>&1 && [[ ! -x "${cli}" ]]; then
    warn "the '${cli}' CLI was not found on PATH. Install and log in to it first \
(the daemon shells it for every remote operation). Continuing — later steps will fail clearly if it \
is missing."
  fi
}

# ---- build + install binaries -----------------------------------------------------------------

install_engine_binaries() {
  if [[ "${no_build}" == "true" ]]; then
    step "Skipping build (--no-build); expecting proton-syncd and proton-sync on PATH"
    return
  fi
  step "Building and installing the engine binaries (proton-syncd, proton-sync)"
  cargo install --path "${repo_root}" --locked --force
}

install_gui_binary() {
  if [[ "${no_build}" == "true" ]]; then
    step "Skipping GUI build (--no-build); expecting proton-sync-gui on PATH"
    return
  fi
  ensure_gui_build_deps
  step "Building and installing the desktop app (proton-sync-gui) — this can take a few minutes"
  cargo install --path "${repo_root}/gui/src-tauri" --locked --force
}

# The Tauri shell links against these at build time. We only *detect* them here (via pkg-config) and,
# unless --install-deps is set, print the right command for the detected package manager rather than
# invoking sudo behind the user's back.
ensure_gui_build_deps() {
  require_command pkg-config "needed to detect the desktop app's system build dependencies"
  local missing=()
  local pkg
  for pkg in webkit2gtk-4.1 libsoup-3.0 gtk+-3.0 glib-2.0; do
    if ! pkg-config --exists "${pkg}" 2>/dev/null; then
      missing+=("${pkg}")
    fi
  done
  if ((${#missing[@]} == 0)); then
    return
  fi

  step "Missing desktop-app build dependencies: ${missing[*]}"
  local manager install_cmd
  if command -v dnf >/dev/null 2>&1; then
    manager=dnf
    install_cmd="sudo dnf install webkit2gtk4.1-devel libsoup3-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel"
  elif command -v apt-get >/dev/null 2>&1; then
    manager=apt
    install_cmd="sudo apt-get install libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev"
  elif command -v pacman >/dev/null 2>&1; then
    manager=pacman
    install_cmd="sudo pacman -S --needed webkit2gtk-4.1 libsoup3 gtk3 libayatana-appindicator librsvg"
  else
    manager=unknown
    install_cmd=""
  fi

  if [[ "${install_deps}" == "true" && "${manager}" != "unknown" ]]; then
    note "Installing with: ${install_cmd}"
    # shellcheck disable=SC2086
    ${install_cmd} || err "dependency install failed; install them manually and re-run"
    return
  fi

  if [[ "${manager}" == "unknown" ]]; then
    err "install the desktop app's WebKitGTK/GTK/libsoup development packages for your distro, then re-run (or use the terminal setup instead of --gui)"
  fi
  err "install the build dependencies first:
    ${install_cmd}
then re-run, or pass --install-deps to have setup.sh run that command for you."
}

# Resolve an installed binary's absolute path — it must point at where the binary actually landed,
# not a guessed default (matters for a custom CARGO_HOME/CARGO_INSTALL_ROOT or --no-build). With
# --no-build we take it from PATH; otherwise from the cargo install dir, falling back to PATH so an
# unusual install root (e.g. a `[install] root` in cargo config) is still found.
resolve_installed_bin() {
  local name="$1" candidate=""
  if [[ "${no_build}" != "true" ]]; then
    candidate="$(cargo_bin_dir)/${name}"
  fi
  if [[ ! -x "${candidate}" ]]; then
    candidate="$(command -v "${name}" 2>/dev/null || true)"
  fi
  [[ -n "${candidate}" && -x "${candidate}" ]] \
    || err "${name} was not found (looked in $(cargo_bin_dir) and on PATH) — did the build fail?"
  printf '%s\n' "${candidate}"
}

resolve_daemon_bin() {
  resolve_installed_bin proton-syncd
}

resolve_gui_bin() {
  resolve_installed_bin proton-sync-gui
}

# ---- config generation (terminal path only) ---------------------------------------------------

write_config() {
  local target="$1"
  local dir
  dir="$(dirname -- "${target}")"
  install -d -m 700 "${dir}"

  if [[ -e "${target}" && "${force_config}" != "true" ]]; then
    note "Keeping existing config: ${target} (pass --force-config to overwrite)"
    # A silent keep here would strand a user who re-ran with a *different* folder pair: the service
    # would keep syncing the old one. Surface the mismatch loudly.
    local existing_local existing_remote
    existing_local="$(config_value "${target}" local_root)"
    existing_remote="$(config_value "${target}" remote_root)"
    if [[ (-n "${existing_local}" && "${existing_local}" != "${local_root}") \
       || (-n "${existing_remote}" && "${existing_remote}" != "${remote_root}") ]]; then
      warn "the existing config syncs '${existing_local:-?}' <-> '${existing_remote:-?}', NOT the \
'${local_root}' <-> '${remote_root}' you just requested."
      warn "the service will keep the EXISTING pair. Re-run with --force-config to replace it."
    fi
    return
  fi

  local proton_cli_line="# proton_cli = \"proton-drive\"    # set if the CLI is not on PATH, or a custom path"
  if [[ -n "${proton_cli}" ]]; then
    proton_cli_line="proton_cli = \"${proton_cli}\""
  fi

  local tmp
  tmp="$(mktemp)"
  cat >"${tmp}" <<EOF
# Proton Drive Sync configuration — generated by setup.sh.
# The systemd user service runs: proton-syncd --config "${target}"
# After editing, apply changes with:  systemctl --user restart proton-syncd

local_root = "${local_root}"
remote_root = "${remote_root}"

${proton_cli_line}
scan_interval_secs = 300

# Selective sync (paths relative to the roots). Uncomment to use:
# include = ["Documents/**", "Projects/**/*.md"]
# exclude = ["**/*.tmp", "**/.DS_Store"]

# Preview only — print a plan and exit instead of syncing:
# dry_run = true

# Delete-approval guard (withholds deletions pending approval; ON both directions by default).
# [delete_approval]
# remote = true
# local = true
EOF
  install -m 600 "${tmp}" "${target}"
  rm -f "${tmp}"
  note "Wrote config (mode 0600): ${target}"
}

# ---- systemd unit -----------------------------------------------------------------------------

install_unit() {
  local daemon_bin="$1"
  local cfg="$2"
  local unit_dir unit_path tmp
  unit_dir="$(config_home)/systemd/user"
  unit_path="${unit_dir}/proton-syncd.service"
  install -d -m 755 "${unit_dir}"

  tmp="$(mktemp)"
  cat >"${tmp}" <<EOF
[Unit]
Description=Proton Drive Sync Daemon
# No After=network-online.target: this is a systemd *user* unit and that target lives in the system
# manager, so ordering against it has no effect. The daemon retries on its own (Restart below) and
# the proton-drive CLI it shells handles transient connectivity.

[Service]
Type=simple
Environment=RUST_LOG=info
ExecStart="${daemon_bin}" --config "${cfg}"
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
EOF
  install -m 644 "${tmp}" "${unit_path}"
  rm -f "${tmp}"
  note "Installed systemd user unit: ${unit_path}"

  systemctl --user daemon-reload
  note "Reloaded the systemd user manager"
}

# ---- desktop launcher (GUI path) --------------------------------------------------------------

install_desktop_launcher() {
  local gui_bin="$1"   # absolute path to the GUI binary (the exec target)
  local path_dir="$2"  # dir to prepend to PATH so the GUI finds proton-syncd / proton-drive
  local apps_dir icon_src icon_dir desktop_path tmp
  apps_dir="$(xdg_data_home)/applications"
  icon_dir="$(xdg_data_home)/icons/hicolor/scalable/apps"
  desktop_path="${apps_dir}/app.protondrivesync.engine.desktop"
  install -d -m 755 "${apps_dir}" "${icon_dir}"

  icon_src="${repo_root}/assets/icon.svg"
  if [[ -f "${icon_src}" ]]; then
    install -m 644 "${icon_src}" "${icon_dir}/app.protondrivesync.engine.svg"
  fi

  # Wrap the exec so a launch from the desktop icon (which runs under the desktop session's PATH,
  # where ~/.cargo/bin usually is not) still finds proton-syncd / proton-drive, which the GUI shells
  # by bare name. The exec target is the GUI's *resolved* absolute path (proton-syncd and the GUI can
  # live in different dirs under --no-build), and path_dir (proton-syncd's dir) is prepended to PATH.
  tmp="$(mktemp)"
  cat >"${tmp}" <<EOF
[Desktop Entry]
Type=Application
Version=1.5
Name=Proton Drive Sync
GenericName=File Synchronization
Comment=Two-way sync between a local folder and Proton Drive
Icon=app.protondrivesync.engine
Exec=sh -c 'PATH="${path_dir}:\$PATH" exec "${gui_bin}"'
Terminal=false
Categories=Network;FileTransfer;
Keywords=proton;drive;sync;backup;cloud;files;
StartupNotify=true
StartupWMClass=app.protondrivesync.engine
EOF
  install -m 644 "${tmp}" "${desktop_path}"
  rm -f "${tmp}"
  note "Installed desktop launcher: ${desktop_path}"
}

xdg_data_home() {
  printf '%s\n' "${XDG_DATA_HOME:-${HOME}/.local/share}"
}

# ---- dry-run gate (terminal path) -------------------------------------------------------------

extract_count() {
  # Pull an integer summary field out of the dry-run JSON. Pinned to the exact key so it can't drift.
  local json="$1" field="$2"
  # `|| true`: a missing field (or head closing the pipe early under `pipefail`) must yield an empty
  # string, never abort the script.
  printf '%s' "${json}" \
    | grep -o "\"${field}\"[[:space:]]*:[[:space:]]*[0-9]\+" \
    | grep -o '[0-9]\+$' \
    | head -n1 || true
}

preview_and_start() {
  local daemon_bin="$1"
  local cfg="$2"

  step "Previewing the plan (dry-run — nothing is changed)"
  local json status=0 errfile
  errfile="$(mktemp)"
  json="$("${daemon_bin}" --config "${cfg}" --dry-run 2>"${errfile}")" || status=$?
  if ((status != 0)); then
    warn "the dry-run preview failed — NOT starting the service."
    sed 's/^/    /' "${errfile}" >&2 || true
    rm -f "${errfile}"
    err "fix the above (commonly: run 'proton-drive login'), then re-run ./setup.sh"
  fi
  rm -f "${errfile}"

  local destructive uploads downloads
  destructive="$(extract_count "${json}" destructive_actions)"
  uploads="$(extract_count "${json}" uploads)"
  downloads="$(extract_count "${json}" downloads)"
  note "Plan summary: uploads=${uploads:-?} downloads=${downloads:-?} destructive_actions=${destructive:-?}"

  if [[ "${no_start}" == "true" ]]; then
    step "Leaving the service stopped (--no-start)"
    note "Start it when ready with:  systemctl --user enable --now proton-syncd"
    return
  fi

  if [[ "${destructive:-0}" != "0" ]]; then
    warn "this plan includes ${destructive} destructive action(s) (deletions/overwrites). Review carefully."
    note "For the full plan run:  ${daemon_bin##*/} --config ${cfg} --dry-run"
  fi

  if [[ "${assume_yes}" != "true" ]]; then
    if [[ -t 0 ]]; then
      local reply
      read -r -p "Start syncing now with these folders? [y/N] " reply
      if [[ ! "${reply}" =~ ^[Yy]$ ]]; then
        step "Not started"
        note "Start it when ready with:  systemctl --user enable --now proton-syncd"
        return
      fi
    else
      step "Non-interactive and no --yes given — not starting the service"
      note "Start it when ready with:  systemctl --user enable --now proton-syncd"
      return
    fi
  fi

  step "Enabling and starting proton-syncd"
  systemctl --user enable --now proton-syncd.service
  note "Service started. Check it with:"
  note "  proton-sync status"
  note "  systemctl --user status proton-syncd"
  note "  journalctl --user -u proton-syncd -f"
}

# ---- GUI path finish --------------------------------------------------------------------------

launch_gui() {
  local gui_bin="$1"   # absolute path to the resolved GUI binary
  local path_dir="$2"  # dir to prepend to PATH so the GUI finds proton-syncd / proton-drive

  if [[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]]; then
    step "No graphical display detected — not launching the desktop app"
    note "Open it from your applications menu (\"Proton Drive Sync\"), or run:  ${gui_bin}"
    return
  fi

  step "Opening the desktop app — finish setup in the onboarding wizard"
  note "Pick your folders, review the dry-run, and press Start. The service then runs in the background."
  # Detach so this terminal is freed; keep the install dir on PATH for the GUI's shell-outs. Prefer
  # setsid (fully detaches from the session) and fall back to nohup where it is unavailable.
  if command -v setsid >/dev/null 2>&1; then
    PATH="${path_dir}:${PATH}" setsid "${gui_bin}" >/dev/null 2>&1 < /dev/null &
  else
    PATH="${path_dir}:${PATH}" nohup "${gui_bin}" >/dev/null 2>&1 < /dev/null &
  fi
  disown 2>/dev/null || true
}

# ---- main -------------------------------------------------------------------------------------

run_terminal_setup() {
  if [[ -z "${config_path}" ]]; then
    config_path="$(default_config_path)"
  fi

  # Resolve the folder pair (flags, else prompt on a TTY, else fail).
  if [[ -z "${local_root}" ]]; then
    if [[ -t 0 ]]; then
      read -r -p "Local folder to sync (e.g. ~/ProtonDrive): " local_root
    fi
    [[ -n "${local_root}" ]] || err "missing --local-root (the local folder to sync)"
  fi
  if [[ -z "${remote_root}" ]]; then
    if [[ -t 0 ]]; then
      read -r -p "Proton Drive folder (e.g. /Drive/RemoteFolder): " remote_root
    fi
    [[ -n "${remote_root}" ]] || err "missing --remote-root (the Proton Drive path)"
  fi

  local_root="$(expand_tilde "${local_root}")"
  step "Preparing local folder: ${local_root}"
  install -d -m 755 "${local_root}"
  # Store an absolute local_root so the service (whatever its working directory) resolves it right.
  local_root="$(cd -- "${local_root}" && pwd)"

  install_engine_binaries
  local daemon_bin
  daemon_bin="$(resolve_daemon_bin)"

  write_config "${config_path}"
  install_unit "${daemon_bin}" "${config_path}"
  preview_and_start "${daemon_bin}" "${config_path}"

  step "Done"
}

run_gui_setup() {
  # In GUI mode the onboarding wizard owns the folder pair, and it writes to the GUI's own config
  # path convention — so ignore a custom --config and any folder flags, and align the unit to it.
  if [[ -n "${config_path}" ]]; then
    warn "--config is ignored with --gui: the onboarding wizard writes to $(default_config_path)"
  fi
  if [[ -n "${local_root}" || -n "${remote_root}" ]]; then
    warn "--local-root/--remote-root are ignored with --gui: choose the folders in the wizard"
  fi
  config_path="$(default_config_path)"

  install_engine_binaries
  install_gui_binary
  # Resolve each binary independently (they can land in different dirs under --no-build); the GUI's
  # shell-outs need proton-syncd's dir on PATH, and the launcher's exec target is the GUI's own path.
  local daemon_bin gui_bin daemon_dir
  daemon_bin="$(resolve_daemon_bin)"
  gui_bin="$(resolve_gui_bin)"
  daemon_dir="$(dirname -- "${daemon_bin}")"

  # Point the unit at the config path the GUI will write, reload so the wizard's Start button works,
  # and enable for reboot persistence. Do NOT start here — the daemon has no config until the wizard
  # writes one; the wizard's step 4 starts it.
  install_unit "${daemon_bin}" "${config_path}"
  systemctl --user enable proton-syncd.service >/dev/null 2>&1 \
    && note "Enabled proton-syncd for future logins (started later from the wizard)" \
    || warn "could not enable proton-syncd; you can start it from the wizard regardless"

  install_desktop_launcher "${gui_bin}" "${daemon_dir}"
  launch_gui "${gui_bin}" "${daemon_dir}"

  step "Done — finish in the desktop app"
}

main() {
  parse_args "$@"

  script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="${script_dir}"
  [[ -f "${repo_root}/Cargo.toml" ]] || err "run setup.sh from the proton-drive-sync-engine checkout (Cargo.toml not found at ${repo_root})"

  check_common_prereqs

  if [[ "${gui_mode}" == "true" ]]; then
    run_gui_setup
  else
    run_terminal_setup
  fi
}

main "$@"
