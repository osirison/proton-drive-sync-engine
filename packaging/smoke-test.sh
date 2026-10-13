#!/usr/bin/env bash
# Install smoke test for a built package (openspec change add-linux-release-pipeline, design D13).
#
#   packaging/smoke-test.sh <upstream version> rpm|deb
#   packaging/smoke-test.sh <upstream version> arch <pacman -U output> <second pacman -U output>
#
# Runs INSIDE a clean container, after the distribution's package manager has installed the main
# package. It looks at what the install left and runs the binaries only with `--version` (and
# `proton-sync doctor`, which works without the daemon); it never starts the daemon or the GUI. It
# is not a test of the repository, so it lives outside `tests/`: run it nowhere but in a throwaway
# container (the checks below look for files a real user's machine would have for real).
#
# Every check runs and every failure is printed, then the exit status is 1; 0 only when all pass.
#
# Test seams (tests/scripts.rs): SMOKE_ROOT is prefixed to every absolute path the script reads or
# runs, and SMOKE_PROC replaces /proc. Nothing else is overridable.
set -uo pipefail

# SMOKE_PROC marks a run by tests/scripts.rs. There an empty SMOKE_ROOT would run the host's real
# binaries, so it is an error before anything runs.
if [[ -n "${SMOKE_PROC:-}" && -z "${SMOKE_ROOT:-}" ]]; then
  echo "smoke-test.sh: SMOKE_PROC is set but SMOKE_ROOT is empty: refusing to run the host's binaries" >&2
  exit 64
fi

root="${SMOKE_ROOT:-}"
proc="${SMOKE_PROC:-/proc}"
cli_url="https://proton.me/drive/download"
failures=0

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

ok() {
  echo "ok: $*"
}

usage() {
  echo "usage: smoke-test.sh <upstream version> rpm|deb" >&2
  echo "       smoke-test.sh <upstream version> arch <pacman -U output> <second pacman -U output>" >&2
  exit 64
}

[[ $# -ge 2 ]] || usage
version="$1"
family="$2"
case "$family" in
  rpm | deb) [[ $# -eq 2 ]] || usage ;;
  arch) [[ $# -eq 4 ]] || usage ;;
  *) usage ;;
esac

# 1. Both CLIs print the release version. `--version` only: nothing else is run on the daemon.
for binary in proton-syncd proton-sync; do
  path="$root/usr/bin/$binary"
  if [[ ! -x "$path" ]]; then
    fail "/usr/bin/$binary is not installed and executable"
    continue
  fi
  printed="$("$path" --version 2>&1)" || true
  if [[ "$printed" == "$binary $version" ]]; then
    ok "$binary --version prints '$printed'"
  else
    fail "$binary --version printed '$printed', expected '$binary $version'"
  fi
done

# 2. The rest of what the main package installs (spec: "What a package installs").
gui="$root/usr/bin/proton-sync-gui"
[[ -x "$gui" ]] || fail "/usr/bin/proton-sync-gui is not installed and executable"
paths=(
  usr/share/applications/app.protondrivesync.engine.desktop
  usr/share/metainfo/app.protondrivesync.engine.metainfo.xml
  usr/share/icons/hicolor/256x256/apps/app.protondrivesync.engine.png
  usr/share/icons/hicolor/scalable/apps/app.protondrivesync.engine.svg
)
case "$family" in
  # %license / PKGBUILD install the three texts here; a deb has its copyright file instead.
  rpm | arch)
    paths+=(
      usr/share/licenses/proton-drive-sync-engine/LICENSE
      usr/share/licenses/proton-drive-sync-engine/OFL-instrument-sans.txt
      usr/share/licenses/proton-drive-sync-engine/OFL-ibm-plex-mono.txt
    )
    ;;
  deb) paths+=(usr/share/doc/proton-drive-sync-engine/copyright) ;;
esac
for path in "${paths[@]}"; do
  [[ -f "$root/$path" ]] || fail "/$path is not installed"
done

# 3. The user unit runs the daemon from /usr/bin.
unit="$root/usr/lib/systemd/user/proton-syncd.service"
if [[ ! -f "$unit" ]]; then
  fail "/usr/lib/systemd/user/proton-syncd.service is not installed"
elif grep -Eq '^ExecStart=/usr/bin/proton-syncd( |$)' "$unit"; then
  ok "the user unit runs /usr/bin/proton-syncd"
else
  fail "the user unit's ExecStart does not begin with /usr/bin/proton-syncd: $(grep '^ExecStart=' "$unit" || echo 'no ExecStart')"
fi

# 4. Installing does not enable the unit for anyone (a deb postinst enables it unless the unit was
# installed with --no-enable). Any wants/requires link to it, in any of the places systemd reads
# one, counts (the spec's /etc/systemd/user/default.target.wants/ is under the first directory).
search=()
for directory in etc/systemd etc/xdg/systemd usr/lib/systemd; do
  [[ -d "$root/$directory" ]] && search+=("$root/$directory")
done
enabled=""
if ((${#search[@]})); then
  enabled="$(find "${search[@]}" \( -path '*.wants/proton-syncd.service' -o -path '*.requires/proton-syncd.service' \) 2>/dev/null)"
fi
if [[ -n "$enabled" ]]; then
  fail "the install enabled the unit: ${enabled//$'\n'/ }"
else
  ok "the unit is installed and not enabled"
fi

# 5. Every shared library the GUI links resolves. ldd loads no program code of the GUI.
if [[ -x "$gui" ]]; then
  # A non-zero ldd (for example "not a dynamic executable") fails too, whatever it printed.
  status=0
  linked="$(ldd "$gui" 2>&1)" || status=$?
  if ((status == 0)) && ! grep -q 'not found' <<<"$linked"; then
    ok "ldd proton-sync-gui resolves every library"
  else
    fail "ldd proton-sync-gui (exit $status): $(tr -s ' \t\n' ' ' <<<"$linked" | cut -c1-300)"
  fi
fi

# 6. The helper tools the daemon runs come with the package (its Recommends / depends).
for tool in curl secret-tool; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is not on PATH"
done

# 7. The freedesktop files validate, where the tool is installed.
if command -v desktop-file-validate >/dev/null 2>&1; then
  if desktop-file-validate "$root/usr/share/applications/app.protondrivesync.engine.desktop"; then
    ok "desktop-file-validate accepts the desktop entry"
  else
    fail "desktop-file-validate rejects the desktop entry"
  fi
else
  echo "skipped: desktop-file-validate is not installed"
fi
if command -v appstreamcli >/dev/null 2>&1; then
  if appstreamcli validate --no-net "$root/usr/share/metainfo/app.protondrivesync.engine.metainfo.xml"; then
    ok "appstreamcli accepts the metainfo"
  else
    fail "appstreamcli rejects the metainfo"
  fi
else
  echo "skipped: appstreamcli is not installed"
fi

# 8. Arch only: pacman's install note appears on the first install and not on the second (a
# reinstall runs post_upgrade).
if [[ "$family" == arch ]]; then
  first="$3"
  second="$4"
  for file in "$first" "$second"; do
    [[ -f "$file" ]] || fail "$file: no such pacman output"
  done
  if [[ -f "$first" ]]; then
    grep -qF "$cli_url" "$first" || fail "pacman -U did not print $cli_url"
    grep -qF 'proton-drive auth login' "$first" || fail "pacman -U did not print 'proton-drive auth login'"
  fi
  if [[ -f "$second" ]] && grep -qE "$cli_url|proton-drive auth login" "$second"; then
    fail "the install note was printed again on a reinstall"
  fi
fi

# 9. The dependency check, with no proton-drive and no daemon (task 3.10): it names proton-drive as
# missing, prints Proton's download URL and exits 1 (a required problem). It must not start the
# daemon or leave a socket. What it says about the keyring is not checked: a container has no
# session bus. HOME and the XDG directories are a scratch directory so nothing real is read.
running() {
  local comm
  for comm in "$proc"/[0-9]*/comm; do
    [[ -r "$comm" ]] || continue
    [[ "$(<"$comm")" == proton-syncd ]] && return 0
  done
  return 1
}

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home" "$scratch/run" "$scratch/cache" "$scratch/config" "$scratch/data" "$scratch/state"
chmod 700 "$scratch/run"

if command -v proton-drive >/dev/null 2>&1; then
  fail "the doctor check needs a container without proton-drive, but $(command -v proton-drive) exists"
elif running; then
  fail "a proton-syncd process is already running before the doctor check"
elif [[ ! -x "$root/usr/bin/proton-sync" ]]; then
  echo "skipped: no /usr/bin/proton-sync to run doctor with (failed above)"
else
  status=0
  before=$failures
  report="$(
    env -u DBUS_SESSION_BUS_ADDRESS -u PROTON_DRIVE_CREDENTIALS_STORE \
      HOME="$scratch/home" XDG_RUNTIME_DIR="$scratch/run" XDG_CACHE_HOME="$scratch/cache" \
      XDG_CONFIG_HOME="$scratch/config" XDG_DATA_HOME="$scratch/data" XDG_STATE_HOME="$scratch/state" \
      timeout 120 "$root/usr/bin/proton-sync" doctor 2>&1
  )" || status=$?
  if [[ "$status" != 1 ]]; then
    fail "proton-sync doctor exited $status, expected 1"
  fi
  if ! grep -qF "$cli_url" <<<"$report"; then
    fail "proton-sync doctor did not print $cli_url"
  fi
  # The real report line is `  missing  proton-drive  proton-drive was not found on PATH` (no
  # colour when not a terminal): the status word and the check id, matched as written.
  if ! grep -Eq '^ *missing +proton-drive( |$)' <<<"$report"; then
    fail "proton-sync doctor did not report proton-drive as missing"
  fi
  if running; then
    fail "a proton-syncd process exists after proton-sync doctor"
  fi
  if [[ -n "$(find "$scratch" -type s 2>/dev/null)" || -n "$(find "$scratch/run" -mindepth 1 2>/dev/null)" ]]; then
    fail "proton-sync doctor left a socket or a socket directory: $(find "$scratch" -type s -o -path "$scratch/run/*" 2>/dev/null | paste -sd ' ')"
  fi
  if ((failures == before)); then
    ok "proton-sync doctor reports proton-drive missing (exit $status), names $cli_url, started no daemon"
  fi
  echo "--- proton-sync doctor ---"
  echo "$report"
  echo "--------------------------"
fi

if ((failures)); then
  echo "smoke test: $failures check(s) failed" >&2
  exit 1
fi
echo "smoke test: all checks passed"
