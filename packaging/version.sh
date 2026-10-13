#!/usr/bin/env bash
# The one place the product version is written, mapped to each package format and checked.
#
#   packaging/version.sh check            compare every version site with the root Cargo.toml
#   packaging/version.sh set <upstream>   write every version site
#
# The upstream version is X.Y.Z for a final release or X.Y.Z-P.N for a pre-release, where P is
# alpha, beta or rc and N is a positive integer. The git tag is "v" plus the upstream version.
# Each package format needs its own spelling, because "~" cannot be in a tag and Arch cannot use
# "-" in a version. For 0.1.0-beta.1:
#
#   Cargo, npm, tauri, the PKGBUILD's _upstream_ver, the release tarball   0.1.0-beta.1
#   RPM Version and AppStream <release>                                    0.1.0~beta.1
#   Debian changelog                                                       0.1.0~beta.1-1
#   Arch pkgver                                                            0.1.0beta1
#
# Each of those sorts below 0.1.0 in its own format's version comparison, and beta.9 below
# beta.10 below rc.1. The mapping below is the only copy of these rules: do not repeat it in a
# workflow, a test or another script, and do not edit a version by hand.
#
# `check` reads files only (grep, sed, awk). `set` also runs `cargo update --workspace --offline`
# and `npm version`, because Cargo.lock and gui/package-lock.json belong to those tools: editing
# them with sed leaves a lockfile that the next `--locked` build refuses.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

readonly SHAPE='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$'
readonly PACKAGE=proton-drive-sync-engine

readonly MANIFESTS=(Cargo.toml gui/gui-core/Cargo.toml gui/src-tauri/Cargo.toml)
readonly LOCK_PACKAGES=(proton-drive-sync-engine proton-sync-gui proton-sync-gui-core)
readonly TAURI_CONF=gui/src-tauri/tauri.conf.json
readonly GUI_PACKAGE=gui/package.json
readonly GUI_LOCK=gui/package-lock.json
readonly SPEC=packaging/rpm/proton-drive-sync-engine.spec
readonly PKGBUILD=packaging/arch/PKGBUILD
readonly DEB_CHANGELOG=packaging/debian/changelog
readonly METAINFO=packaging/freedesktop/app.protondrivesync.engine.metainfo.xml
readonly ASSETS=examples/packaging/release-assets.toml

usage() {
  cat >&2 <<'EOF'
usage: packaging/version.sh check
       packaging/version.sh set <upstream version>

The upstream version is X.Y.Z or X.Y.Z-(alpha|beta|rc).N, for example 0.1.0 or 0.1.0-beta.1.
EOF
}

die() {
  echo "version.sh: $*" >&2
  exit 1
}

valid_shape() {
  [[ "$1" =~ $SHAPE ]]
}

# --- The mapping -------------------------------------------------------------------------------

rpm_version() { printf '%s' "${1//-/\~}"; }
deb_version() { printf '%s-1' "$(rpm_version "$1")"; }
# Arch: the pre-release part loses its "-" and its ".", so beta.1 is beta1 and sorts by vercmp.
arch_version() {
  local core="${1%%-*}" pre=""
  [[ "$1" == *-* ]] && pre="${1#*-}"
  printf '%s%s' "$core" "${pre//./}"
}
appstream_type() {
  if [[ "$1" == *-* ]]; then printf 'development'; else printf 'stable'; fi
}

# --- Reading the sites -------------------------------------------------------------------------

# version of the [package] table of a Cargo manifest
manifest_version() {
  awk '
    /^\[/ { in_package = ($0 == "[package]") }
    in_package && /^version[ ]*=/ { sub(/^version[ ]*=[ ]*"/, ""); sub(/".*/, ""); print; exit }
  ' "$root/$1"
}

# version of one workspace package in Cargo.lock
lock_version() {
  awk -v want="$1" '
    /^\[\[package\]\]/ { name = "" }
    /^name = / { name = $3; gsub(/"/, "", name) }
    name == want && /^version = / { v = $3; gsub(/"/, "", v); print v; exit }
  ' "$root/Cargo.lock"
}

# the top-level "version" of a JSON file that npm or Tauri formats with two-space indentation
json_version() {
  sed -n -E 's/^  "version": "([^"]*)".*/\1/p' "$root/$1" | head -n 1
}

# the version of the root package entry ("") of package-lock.json
npm_lock_root_version() {
  awk '
    /^    "": \{/ { in_root = 1 }
    in_root && /"version":/ { sub(/^[^:]*:[ ]*"/, ""); sub(/".*/, ""); print; exit }
  ' "$root/$GUI_LOCK"
}

# NAME in "NAME value" at the start of a line of a spec or a PKGBUILD, e.g. "Version:" or "pkgver="
line_value() {
  local file="$1" prefix="$2"
  grep -m1 -E "^${prefix}" "$root/$file" | sed -E "s/^${prefix}[[:space:]]*//; s/[[:space:]]*\$//" || true
}

# the text after the last " - " of the first entry under %changelog
spec_changelog_version() {
  awk '/^%changelog/ { on = 1; next } on && /^\* / { print; exit }' "$root/$SPEC" | sed -E 's/.* - //'
}

deb_changelog_version() {
  sed -n -E '1s/^[^ ]+ \(([^)]*)\).*/\1/p' "$root/$DEB_CHANGELOG"
}

# attribute $1 of the first <release> element
metainfo_release_attribute() {
  grep -m1 -E '^[[:space:]]*<release ' "$root/$METAINFO" |
    sed -n -E "s/.*[[:space:]]$1=\"([^\"]*)\".*/\1/p"
}

assets_archive_name() {
  sed -n -E 's/^default_archive[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p' "$root/$ASSETS" | sed -E 's|.*/||'
}

# --- check -------------------------------------------------------------------------------------

failures=0
compared=0

# expect FILE WHAT EXPECTED FOUND
expect() {
  compared=$((compared + 1))
  if [[ "$3" != "$4" ]]; then
    printf '%s: %s: expected %s, found %s\n' "$1" "$2" "${3:-<nothing>}" "${4:-<nothing>}" >&2
    failures=$((failures + 1))
  fi
}

# expect_revision FILE WHAT EXPECTED_VERSION FOUND
# A changelog entry is VERSION-REVISION. The version must match exactly; the revision may be any
# positive integer, so a packaging-only rebuild (-2) is a change to the entry and not to the
# product version.
expect_revision() {
  local file="$1" what="$2" want="$3" found="$4"
  compared=$((compared + 1))
  if [[ "$found" =~ ^(.+)-([1-9][0-9]*)$ && "${BASH_REMATCH[1]}" == "$want" ]]; then
    return 0
  fi
  printf '%s: %s: expected %s-<revision>, found %s\n' "$file" "$what" "$want" "${found:-<nothing>}" >&2
  failures=$((failures + 1))
}

cmd_check() {
  local upstream rpm arch name
  upstream="$(manifest_version Cargo.toml)"
  valid_shape "$upstream" || die "Cargo.toml: version '$upstream' is not X.Y.Z or X.Y.Z-(alpha|beta|rc).N"
  rpm="$(rpm_version "$upstream")"
  arch="$(arch_version "$upstream")"

  local file
  for file in "${MANIFESTS[@]}"; do
    expect "$file" "package version" "$upstream" "$(manifest_version "$file")"
  done
  for name in "${LOCK_PACKAGES[@]}"; do
    expect Cargo.lock "version of $name" "$upstream" "$(lock_version "$name")"
  done
  expect "$TAURI_CONF" "version" "$upstream" "$(json_version "$TAURI_CONF")"
  expect "$GUI_PACKAGE" "version" "$upstream" "$(json_version "$GUI_PACKAGE")"
  expect "$GUI_LOCK" "version" "$upstream" "$(json_version "$GUI_LOCK")"
  expect "$GUI_LOCK" "version of the root package" "$upstream" "$(npm_lock_root_version)"

  expect "$SPEC" "%global upstream_version" "$upstream" "$(line_value "$SPEC" '%global[[:space:]]+upstream_version')"
  expect "$SPEC" "Version" "$rpm" "$(line_value "$SPEC" 'Version:')"
  expect_revision "$SPEC" "newest %changelog entry" "$rpm" "$(spec_changelog_version)"

  expect "$PKGBUILD" "_upstream_ver" "$upstream" "$(line_value "$PKGBUILD" '_upstream_ver=')"
  expect "$PKGBUILD" "pkgver" "$arch" "$(line_value "$PKGBUILD" 'pkgver=')"

  expect_revision "$DEB_CHANGELOG" "newest entry" "$rpm" "$(deb_changelog_version)"

  expect "$METAINFO" "newest <release> version" "$rpm" "$(metainfo_release_attribute version)"
  expect "$METAINFO" "newest <release> type" "$(appstream_type "$upstream")" "$(metainfo_release_attribute type)"

  expect "$ASSETS" "default_archive file name" "$PACKAGE-$upstream.tar.gz" "$(assets_archive_name)"

  if ((failures > 0)); then
    echo "version.sh: $failures of $compared version values differ from Cargo.toml ($upstream)." >&2
    echo "version.sh: change versions only with: packaging/version.sh set <upstream version>" >&2
    exit 1
  fi
  echo "version.sh: all $compared version values agree with Cargo.toml ($upstream)."
}

# --- set ---------------------------------------------------------------------------------------

# replace_line FILE REGEX REPLACEMENT: replace the first line matching REGEX (sed -E, whole line)
replace_line() {
  local file="$root/$1"
  grep -q -E "$2" "$file" || die "$1: no line matches $2"
  sed -i -E "0,/$2/s|$2|$3|" "$file"
}

# manifest_set FILE VERSION: the version line of the [package] table only
manifest_set() {
  local file="$root/$1"
  awk -v v="$2" '
    /^\[/ { in_package = ($0 == "[package]") }
    in_package && !done && /^version[ ]*=/ { print "version = \"" v "\""; done = 1; next }
    { print }
    END { if (!done) exit 3 }
  ' "$file" >"$file.new" || { rm -f "$file.new"; die "$1: no version in [package]"; }
  command mv -f "$file.new" "$file"
}

json_set() {
  replace_line "$1" '^  "version": "[^"]*"' "  \"version\": \"$2\""
}

today_rpm() { LC_ALL=C date +'%a %b %d %Y'; }
today_rfc2822() { LC_ALL=C date -R; }
today_iso() { LC_ALL=C date +%F; }

add_spec_changelog() {
  local upstream="$1" rpm="$2" newest who file="$root/$SPEC"
  newest="$(spec_changelog_version)"
  [[ "$newest" == "$rpm-"* ]] && return 0
  # keep the name and address of the previous entry: "* Day Mon DD YYYY NAME <ADDRESS> - VERSION"
  who="$(awk '/^%changelog/ { on = 1; next } on && /^\* / { print; exit }' "$file" |
    sed -E 's/^\* [A-Za-z]{3} [A-Za-z]{3} [0-9]{1,2} [0-9]{4} //; s/ - .*//')"
  [[ -n "$who" ]] || die "$SPEC: no earlier %changelog entry to take the name from"
  awk -v entry="* $(today_rpm) $who - $rpm-1" -v note="- Release $upstream." '
    { print }
    /^%changelog/ { print entry; print note; print "" }
  ' "$file" >"$file.new"
  command mv -f "$file.new" "$file"
}

add_deb_changelog() {
  local upstream="$1" rpm="$2" file="$root/$DEB_CHANGELOG" who
  [[ "$(deb_changelog_version)" == "$rpm-"* ]] && return 0
  who="$(grep -m1 -E '^ -- ' "$file" | sed -E 's/^ -- (.*[>])  .*/\1/')"
  [[ -n "$who" ]] || die "$DEB_CHANGELOG: no earlier entry to take the maintainer from"
  {
    printf '%s (%s) unstable; urgency=medium\n\n' "$PACKAGE" "$(deb_version "$upstream")"
    printf '  * Release %s.\n\n' "$upstream"
    printf ' -- %s  %s\n\n' "$who" "$(today_rfc2822)"
    cat "$file"
  } >"$file.new"
  command mv -f "$file.new" "$file"
}

add_metainfo_release() {
  local upstream="$1" rpm="$2" file="$root/$METAINFO"
  [[ "$(metainfo_release_attribute version)" == "$rpm" ]] && return 0
  grep -q -E '^[[:space:]]*<releases>' "$file" || die "$METAINFO: no <releases> element"
  awk -v v="$rpm" -v d="$(today_iso)" -v t="$(appstream_type "$upstream")" -v up="$upstream" '
    { print }
    /^[[:space:]]*<releases>/ {
      print "    <release version=\"" v "\" date=\"" d "\" type=\"" t "\">"
      print "      <description>"
      print "        <p>Release " up ".</p>"
      print "      </description>"
      print "    </release>"
    }
  ' "$file" >"$file.new"
  command mv -f "$file.new" "$file"
}

cmd_set() {
  local upstream="${1:-}"
  [[ $# -eq 1 ]] || { usage; exit 2; }
  if ! valid_shape "$upstream"; then
    echo "version.sh: '$upstream' is not an upstream version. Accepted: X.Y.Z (0.1.0) and" >&2
    echo "version.sh: X.Y.Z-(alpha|beta|rc).N with N a positive integer (0.1.0-beta.1)." >&2
    exit 2
  fi
  local rpm arch file
  rpm="$(rpm_version "$upstream")"
  arch="$(arch_version "$upstream")"

  # Everything that will be written must exist before the first write, so a wrong checkout
  # changes nothing.
  for file in "${MANIFESTS[@]}" Cargo.lock "$TAURI_CONF" "$GUI_PACKAGE" "$GUI_LOCK" "$SPEC" "$PKGBUILD" \
    "$DEB_CHANGELOG" "$METAINFO" "$ASSETS"; do
    [[ -f "$root/$file" ]] || die "$file: not found under $root"
  done
  command -v cargo >/dev/null || die "cargo is needed to regenerate Cargo.lock"
  command -v npm >/dev/null || die "npm is needed to regenerate gui/package-lock.json"

  for file in "${MANIFESTS[@]}"; do manifest_set "$file" "$upstream"; done
  json_set "$TAURI_CONF" "$upstream"
  json_set "$GUI_PACKAGE" "$upstream"

  replace_line "$SPEC" '^%global[[:space:]]+upstream_version[[:space:]].*' "%global upstream_version $upstream"
  replace_line "$SPEC" '^Version:[[:space:]].*' "Version:        $rpm"
  add_spec_changelog "$upstream" "$rpm"

  replace_line "$PKGBUILD" '^_upstream_ver=.*' "_upstream_ver=$upstream"
  replace_line "$PKGBUILD" '^pkgver=.*' "pkgver=$arch"
  replace_line "$PKGBUILD" '^pkgrel=.*' "pkgrel=1"

  add_deb_changelog "$upstream" "$rpm"
  add_metainfo_release "$upstream" "$rpm"
  replace_line "$ASSETS" '^default_archive[[:space:]]*=.*' "default_archive = \"target/dist/$PACKAGE-$upstream.tar.gz\""

  # The two lockfiles are regenerated by the tools that own them. --offline: the three workspace
  # entries are the only ones that change, and they need no registry.
  (cd "$root" && cargo update --workspace --offline --quiet)
  # npm also rewrites package.json (it turns the "—" in the description into a literal dash),
  # which is noise in a version bump, so the file written above is put back after the lockfile is
  # regenerated. The version in it is the same one.
  command cp -p "$root/$GUI_PACKAGE" "$root/$GUI_PACKAGE.kept"
  (cd "$root/gui" && npm version "$upstream" --no-git-tag-version --allow-same-version --ignore-scripts >/dev/null) ||
    { command mv -f "$root/$GUI_PACKAGE.kept" "$root/$GUI_PACKAGE"; die "npm version failed"; }
  command mv -f "$root/$GUI_PACKAGE.kept" "$root/$GUI_PACKAGE"

  echo "version.sh: set $upstream (RPM $rpm, Debian $(deb_version "$upstream"), Arch $arch)."
}

case "${1:-}" in
  check)
    [[ $# -eq 1 ]] || { usage; exit 2; }
    cmd_check
    ;;
  set)
    shift
    cmd_set "$@"
    ;;
  *)
    usage
    exit 2
    ;;
esac
