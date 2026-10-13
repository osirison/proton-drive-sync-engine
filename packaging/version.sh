#!/usr/bin/env bash
# The one place the product version is written, mapped to each package format and checked.
#
#   packaging/version.sh check            compare every version site with the root Cargo.toml
#   packaging/version.sh set <upstream>   write every version site
#   packaging/version.sh tag-version <tag>   the upstream version a release tag names, or an error
#   packaging/version.sh is-prerelease <v>   prints true or false
#   packaging/version.sh current             the version the root Cargo.toml states
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
# them with sed leaves a lockfile that the next `--locked` build refuses. The offline cargo step
# needs a populated cargo cache (run `cargo fetch` once with the network first); it does not need
# the network itself.
#
# `set` is all-or-nothing: it copies every file it will write first, and if anything fails it puts
# them all back and exits non-zero. It also drops the changelog and AppStream entries of every
# version equal to or higher than the one being set (see "Superseded entries" below), and refuses
# to do so for a version whose tag exists (local tags only: run `git fetch --tags` first). The date
# in a new entry is today's, or the instant in SOURCE_DATE_EPOCH when that is set (the
# reproducible-builds variable, seconds since the epoch, read as UTC).
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
       packaging/version.sh spell <rpm|deb|arch> <upstream version>
       packaging/version.sh tag-version <tag>
       packaging/version.sh is-prerelease <upstream version>
       packaging/version.sh current

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

# The date of an entry is today's, or the instant in SOURCE_DATE_EPOCH (UTC) when that is set, so
# that a run can be repeated byte for byte.
entry_date() {
  if [[ -n "${SOURCE_DATE_EPOCH:-}" ]]; then
    LC_ALL=C TZ=UTC date -d "@$SOURCE_DATE_EPOCH" "$@"
  else
    LC_ALL=C date "$@"
  fi
}
# SOURCE_DATE_EPOCH must be digits that `date` accepts; anything else is refused before a write.
# Unset or empty means today.
check_source_date_epoch() {
  [[ -n "${SOURCE_DATE_EPOCH:-}" ]] || return 0
  if [[ ! "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]] ||
    ! LC_ALL=C TZ=UTC date -d "@$SOURCE_DATE_EPOCH" +%F >/dev/null 2>&1; then
    echo "version.sh: SOURCE_DATE_EPOCH='$SOURCE_DATE_EPOCH' is not a number of seconds since the epoch (digits only)." >&2
    exit 2
  fi
}

# The three spellings of the entry date, read once before the first write. A `date` that fails or
# prints nothing stops the run (and rolls it back) instead of writing an empty date.
DATE_RPM="" DATE_RFC2822="" DATE_ISO=""
read_entry_dates() {
  DATE_RPM="$(today_rpm)" && DATE_RFC2822="$(today_rfc2822)" && DATE_ISO="$(today_iso)" ||
    die "date failed while reading the entry date"
  [[ -n "$DATE_RPM" && -n "$DATE_RFC2822" && -n "$DATE_ISO" ]] || die "date printed nothing for the entry date"
}
today_rpm() { entry_date +'%a %b %d %Y'; }
today_rfc2822() { entry_date -R; }
today_iso() { entry_date +%F; }

add_spec_changelog() {
  local upstream="$1" rpm="$2" revision="$3" who="$4" file="$root/$SPEC"
  awk -v entry="* $DATE_RPM $who - $rpm-$revision" -v note="- Release $upstream." '
    { print }
    /^%changelog/ { print entry; print note; print "" }
  ' "$file" >"$file.new"
  command mv -f "$file.new" "$file"
}

add_deb_changelog() {
  local upstream="$1" rpm="$2" revision="$3" who="$4" file="$root/$DEB_CHANGELOG"
  {
    printf '%s (%s-%s) unstable; urgency=medium\n\n' "$PACKAGE" "$rpm" "$revision"
    printf '  * Release %s.\n\n' "$upstream"
    printf ' -- %s  %s\n\n' "$who" "$DATE_RFC2822"
    cat "$file"
  } >"$file.new"
  command mv -f "$file.new" "$file"
}

add_metainfo_release() {
  local upstream="$1" rpm="$2" file="$root/$METAINFO"
  grep -q -E '^[[:space:]]*<releases>' "$file" || die "$METAINFO: no <releases> element"
  awk -v v="$rpm" -v d="$DATE_ISO" -v t="$(appstream_type "$upstream")" -v up="$upstream" '
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

# The name and address of the newest entry, read before the entries are dropped: the new entry
# is written in the same name.
spec_changelog_author() {
  # "* Day Mon DD YYYY NAME <ADDRESS> - VERSION"
  awk '/^%changelog/ { on = 1; next } on && /^\* / { print; exit }' "$root/$SPEC" |
    sed -E 's/^\* [A-Za-z]{3} [A-Za-z]{3} [0-9]{1,2} [0-9]{4} //; s/ - .*//'
}
deb_changelog_author() {
  grep -m1 -E '^ -- ' "$root/$DEB_CHANGELOG" | sed -E 's/^ -- (.*[>])  .*/\1/' || true
}

# --- Superseded entries ------------------------------------------------------------------------
#
# Setting V removes every changelog and <release> entry whose upstream version is V or higher,
# then adds V's entry on top. An equal entry is replaced, so running `set` again for one version
# changes nothing. A higher entry cannot have been released: the policy is to fix forward, and
# lowering a published version needs an epoch. Leaving it would stack V above or beside entries
# it supersedes (appstream: releases-not-in-order; two 0.1.0 entries after the final release).

# "X Y Z RANK N" for an upstream version; RANK orders alpha < beta < rc < final.
version_key() {
  local v="$1" core pre="" rank=4 n=0 x y z
  core="${v%%-*}"
  if [[ "$v" == *-* ]]; then
    pre="${v#*-}"
    case "${pre%%.*}" in
      alpha) rank=1 ;;
      beta) rank=2 ;;
      rc) rank=3 ;;
    esac
    n="${pre#*.}"
  fi
  IFS=. read -r x y z <<<"$core"
  echo "$x $y $z $rank $n"
}

# prints -1, 0 or 1 for A <, = or > B
version_compare() {
  local -a a b
  local i
  read -ra a <<<"$(version_key "$1")"
  read -ra b <<<"$(version_key "$2")"
  for i in 0 1 2 3 4; do
    if ((a[i] < b[i])); then echo -1; return 0; fi
    if ((a[i] > b[i])); then echo 1; return 0; fi
  done
  echo 0
}

# "ORDINAL<TAB>VERSION-REVISION" for each entry of a file, newest first. KIND: spec, deb, meta.
list_entries() {
  case "$1" in
    spec) awk '/^%changelog/ { on = 1; next } on && /^\* / { n++; v = $0; sub(/.* - /, "", v); print n "\t" v }' "$root/$SPEC" ;;
    deb) awk '/^proton-drive-sync-engine \(/ { n++; v = $0; sub(/^[^(]*\(/, "", v); sub(/\).*/, "", v); print n "\t" v }' "$root/$DEB_CHANGELOG" ;;
    meta) awk '/<release / { n++; v = $0; sub(/.*version="/, "", v); sub(/".*/, "", v); print n "\t" v "-1" }' "$root/$METAINFO" ;;
  esac
}

# drop_entries KIND " 1 3 ": delete the entries with those ordinals, whole
drop_entries() {
  local kind="$1" ordinals="$2" file
  case "$kind" in
    spec) file="$root/$SPEC" ;;
    deb) file="$root/$DEB_CHANGELOG" ;;
    meta) file="$root/$METAINFO" ;;
  esac
  awk -v kind="$kind" -v drop="$ordinals" '
    kind == "spec" && /^%changelog/ { in_log = 1 }
    {
      starts = 0
      if (kind == "spec") starts = (in_log && $0 ~ /^\* /)
      else if (kind == "deb") starts = ($0 ~ /^proton-drive-sync-engine \(/)
      else starts = ($0 ~ /<release /)
      if (starts) { n++; skipping = (index(drop, " " n " ") > 0) }
      if (skipping) {
        # an entry ends at </release>, or on its own line when it is self-closing (<release .../>)
        if (kind == "meta" && ($0 ~ /<\/release>/ || $0 ~ /<release .*\/>[ \t]*$/)) skipping = 0
        next
      }
      print
    }
  ' "$file" >"$file.new"
  command mv -f "$file.new" "$file"
}

# plan_drops KIND UPSTREAM: sets DROP_ORDINALS (" 1 2 "), DROP_VERSIONS (upstream forms) and
# EQUAL_REVISION (the revision of an entry for UPSTREAM itself, or 1)
plan_drops() {
  local kind="$1" upstream="$2" ordinal entry version revision
  DROP_ORDINALS=" "
  DROP_VERSIONS=()
  EQUAL_REVISION=1
  while IFS=$'\t' read -r ordinal entry; do
    [[ -n "$ordinal" ]] || continue
    revision="${entry##*-}"
    version="${entry%-*}"
    version="${version//\~/-}"
    valid_shape "$version" || die "a $kind entry has the version '$entry', which is not an upstream version"
    if [[ "$(version_compare "$version" "$upstream")" != -1 ]]; then
      DROP_ORDINALS+="$ordinal "
      DROP_VERSIONS+=("$version")
      if [[ "$version" == "$upstream" && "$revision" =~ ^[1-9][0-9]*$ ]]; then EQUAL_REVISION="$revision"; fi
    fi
  done < <(list_entries "$kind")
}

# --- Undoing a failed set ----------------------------------------------------------------------

snapshot_dir=""
committed=0

restore_snapshot() {
  local file
  for file in "$@"; do
    command cp -p "$snapshot_dir/$file" "$root/$file"
    rm -f "$root/$file.new" "$root/$file.kept"
  done
}

SET_FILES=()
SNAPSHOTTED=()

# Runs when the script ends, whatever the reason. `set -e` is off inside it so that one file that
# cannot be put back does not stop the others, and the snapshot directory is always removed.
on_exit() {
  local status=$?
  set +e
  if [[ -n "$snapshot_dir" ]]; then
    if ((committed == 0)); then
      if ((${#SNAPSHOTTED[@]} > 0)); then
        restore_snapshot "${SNAPSHOTTED[@]}"
      fi
      echo "version.sh: set failed; every file that was copied is back as it was." >&2
    fi
    rm -rf "$snapshot_dir"
  fi
  return "$status"
}

cmd_set() {
  local upstream="${1:-}"
  [[ $# -eq 1 ]] || { usage; exit 2; }
  if ! valid_shape "$upstream"; then
    echo "version.sh: '$upstream' is not an upstream version. Accepted: X.Y.Z (0.1.0) and" >&2
    echo "version.sh: X.Y.Z-(alpha|beta|rc).N with N a positive integer (0.1.0-beta.1)." >&2
    exit 2
  fi
  check_source_date_epoch
  local rpm arch file kind
  rpm="$(rpm_version "$upstream")"
  arch="$(arch_version "$upstream")"
  SET_FILES=("${MANIFESTS[@]}" Cargo.lock "$TAURI_CONF" "$GUI_PACKAGE" "$GUI_LOCK" "$SPEC" "$PKGBUILD" \
    "$DEB_CHANGELOG" "$METAINFO" "$ASSETS")

  # Everything that will be written must exist before the first write, so a wrong checkout
  # changes nothing.
  for file in "${SET_FILES[@]}"; do
    [[ -f "$root/$file" ]] || die "$file: not found under $root"
  done
  command -v cargo >/dev/null || die "cargo is needed to regenerate Cargo.lock"
  command -v npm >/dev/null || die "npm is needed to regenerate gui/package-lock.json"

  # An entry is found by its <release tag; with two on one line the later one would hide the first
  # (and so its version would never be compared), so refuse rather than guess.
  if awk 'gsub(/<release[ \t>]/, "&") > 1 { found = 1 } END { exit !found }' "$root/$METAINFO"; then
    echo "version.sh: $METAINFO: a line holds more than one <release tag; put each on its own line. Nothing was changed." >&2
    exit 2
  fi

  local spec_who deb_who
  spec_who="$(spec_changelog_author)"
  [[ -n "$spec_who" ]] || die "$SPEC: no %changelog entry to take the name from"
  deb_who="$(deb_changelog_author)"
  [[ -n "$deb_who" ]] || die "$DEB_CHANGELOG: no entry to take the maintainer from"

  # Plan which entries are superseded, and refuse before any write if one of them was tagged.
  local spec_drops deb_drops meta_drops spec_rev deb_rev dropped=()
  plan_drops spec "$upstream"; spec_drops="$DROP_ORDINALS"; spec_rev="$EQUAL_REVISION"
  dropped+=("${DROP_VERSIONS[@]/#/RPM %changelog }")
  plan_drops deb "$upstream"; deb_drops="$DROP_ORDINALS"; deb_rev="$EQUAL_REVISION"
  dropped+=("${DROP_VERSIONS[@]/#/debian\/changelog }")
  plan_drops meta "$upstream"; meta_drops="$DROP_ORDINALS"
  dropped+=("${DROP_VERSIONS[@]/#/metainfo }")
  local root_physical top entry version
  root_physical="$(cd "$root" && pwd -P)"
  local tag_check=yes
  if ! command -v git >/dev/null; then
    tag_check="git is not installed"
  elif ! top="$(git -C "$root" rev-parse --show-toplevel 2>/dev/null)"; then
    tag_check="$root is not in a git repository"
  elif [[ "$top" != "$root_physical" ]]; then
    tag_check="$root is not the top level of its git repository ($top)"
  fi
  if [[ "$tag_check" != yes ]]; then
    # said only when an entry is going to be dropped: with none, there is no tag to look up
    if ((${#dropped[@]} > 0)); then
      echo "version.sh: tag check skipped: $tag_check, so a tag for a dropped version cannot be looked up." >&2
    fi
  else
    # Local tags only: run `git fetch --tags` first, or a tag that exists only on the remote is missed.
    if ((${#dropped[@]} > 0)); then
      echo "version.sh: checking local tags only; run 'git fetch --tags' first if this clone is behind." >&2
    fi
    for entry in "${dropped[@]}"; do
      version="${entry##* }"
      if [[ -n "$(git -C "$root" tag -l "v$version")" ]]; then
        echo "version.sh: refusing: setting $upstream would drop the $entry entry, and the tag v$version exists." >&2
        echo "version.sh: a tagged version cannot be lowered; set a higher one." >&2
        exit 2
      fi
    done
  fi

  # From here on a failure puts every file back.
  snapshot_dir="$(mktemp -d)"
  trap on_exit EXIT
  read_entry_dates
  for file in "${SET_FILES[@]}"; do
    mkdir -p "$snapshot_dir/$(dirname "$file")"
    command cp -p "$root/$file" "$snapshot_dir/$file" || die "$file: cannot be read to take a copy; nothing was changed"
    SNAPSHOTTED+=("$file")
  done

  for entry in "${dropped[@]}"; do
    echo "version.sh: dropping the $entry entry (superseded by $upstream)"
  done
  local current_upstream
  current_upstream="$(line_value "$PKGBUILD" '_upstream_ver=')"

  for file in "${MANIFESTS[@]}"; do manifest_set "$file" "$upstream"; done
  json_set "$TAURI_CONF" "$upstream"
  json_set "$GUI_PACKAGE" "$upstream"

  replace_line "$SPEC" '^%global[[:space:]]+upstream_version[[:space:]].*' "%global upstream_version $upstream"
  replace_line "$SPEC" '^Version:[[:space:]].*' "Version:        $rpm"
  drop_entries spec "$spec_drops"
  add_spec_changelog "$upstream" "$rpm" "$spec_rev" "$spec_who"

  replace_line "$PKGBUILD" '^_upstream_ver=.*' "_upstream_ver=$upstream"
  replace_line "$PKGBUILD" '^pkgver=.*' "pkgver=$arch"
  # A new upstream version starts at revision 1. The same version again keeps a revision that was
  # raised on purpose for a packaging-only rebuild.
  if [[ "$current_upstream" != "$upstream" ]]; then
    replace_line "$PKGBUILD" '^pkgrel=.*' "pkgrel=1"
  fi

  drop_entries deb "$deb_drops"
  add_deb_changelog "$upstream" "$rpm" "$deb_rev" "$deb_who"
  drop_entries meta "$meta_drops"
  add_metainfo_release "$upstream" "$rpm"
  # An entry whose end the script could not see would have taken the closing tags with it.
  grep -q '</releases>' "$root/$METAINFO" && grep -q '</component>' "$root/$METAINFO" ||
    die "$METAINFO: no longer has </releases> and </component> after the entries were rewritten"
  if command -v xmllint >/dev/null; then
    xmllint --noout "$root/$METAINFO" || die "$METAINFO: is no longer well-formed XML"
  fi
  replace_line "$ASSETS" '^default_archive[[:space:]]*=.*' "default_archive = \"target/dist/$PACKAGE-$upstream.tar.gz\""

  # The two lockfiles are regenerated by the tools that own them. --offline: the three workspace
  # entries are the only ones that change, but cargo still reads the registry index and the
  # crates it already has, so the cargo cache must be populated.
  (cd "$root" && cargo update --workspace --offline --quiet) ||
    die "cargo update --workspace --offline failed: its cache is empty or incomplete. Run 'cargo fetch' (needs the network) and then 'packaging/version.sh set $upstream' again."
  # npm also rewrites package.json (it turns the "—" in the description into a literal dash),
  # which is noise in a version bump, so the file written above is put back after the lockfile is
  # regenerated. The version in it is the same one.
  command cp -p "$root/$GUI_PACKAGE" "$root/$GUI_PACKAGE.kept"
  (cd "$root/gui" && npm version "$upstream" --no-git-tag-version --allow-same-version --ignore-scripts >/dev/null) ||
    die "npm version failed"
  command mv -f "$root/$GUI_PACKAGE.kept" "$root/$GUI_PACKAGE"

  committed=1
  echo "version.sh: set $upstream (RPM $rpm, Debian $rpm-$deb_rev, Arch $arch)."
}

# Prints one format's spelling of an upstream version and writes nothing. For a built package's
# name-assets.sh: it asks here what the package's own version must be instead of repeating the
# mapping. The deb spelling carries the revision `set` writes for a new version (-1); a caller
# comparing a built package ignores the revision, which a packaging-only rebuild raises.
cmd_spell() {
  [[ $# -eq 2 ]] || { usage; exit 2; }
  valid_shape "$2" || die "'$2' is not an upstream version (X.Y.Z or X.Y.Z-(alpha|beta|rc).N)"
  case "$1" in
    rpm) rpm_version "$2" ;;
    deb) deb_version "$2" ;;
    arch) arch_version "$2" ;;
    *) die "unknown format '$1' (rpm, deb or arch)" ;;
  esac
  echo
}

# The upstream version a release tag names: "v" plus a version of the accepted shape. The release
# workflow's gate asks this instead of keeping a tag pattern of its own.
cmd_tag_version() {
  [[ $# -eq 1 ]] || { usage; exit 2; }
  if [[ "$1" != v* ]] || ! valid_shape "${1#v}"; then
    die "'$1' is not a release tag: it must be v plus X.Y.Z or X.Y.Z-(alpha|beta|rc).N (for example v0.1.0 or v0.1.0-beta.1)"
  fi
  printf '%s\n' "${1#v}"
}

# Prints true when the version has a pre-release part and false when it has none. The answer is on
# stdout so that a version of the wrong shape (exit 1, nothing printed) is not read as "false".
cmd_is_prerelease() {
  [[ $# -eq 1 ]] || { usage; exit 2; }
  valid_shape "$1" || die "'$1' is not an upstream version (X.Y.Z or X.Y.Z-(alpha|beta|rc).N)"
  if [[ "$1" == *-* ]]; then echo true; else echo false; fi
}

# The version the root Cargo.toml states, which every other site is held to.
cmd_current() {
  [[ $# -eq 0 ]] || { usage; exit 2; }
  local version
  version="$(manifest_version Cargo.toml)"
  [[ -n "$version" ]] || die "Cargo.toml: no version in the [package] table"
  printf '%s\n' "$version"
}

case "${1:-}" in
  tag-version)
    shift
    cmd_tag_version "$@"
    ;;
  is-prerelease)
    shift
    cmd_is_prerelease "$@"
    ;;
  current)
    shift
    cmd_current "$@"
    ;;
  spell)
    shift
    cmd_spell "$@"
    ;;
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
