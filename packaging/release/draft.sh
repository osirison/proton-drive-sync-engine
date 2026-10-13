#!/usr/bin/env bash
# The release workflow's `draft` job (openspec change add-linux-release-pipeline, design D12).
#
#   packaging/release/draft.sh <tag> <assets-dir>
#   packaging/release/draft.sh --check-assets <assets-dir> <upstream version>
#
# The first form needs gh (with GH_TOKEN that may write releases), jq, sha256sum and
# GITHUB_REPOSITORY (owner/name). <assets-dir> holds the built assets and nothing else: the source
# tarball, the source RPM, three RPMs, three debs and the Arch package. This script writes
# SHA256SUMS beside them.
#
# What it does for the tag:
#
#   no release              creates a DRAFT (--verify-tag: the tag must already exist), with the
#                           notes header from packaging/release-notes.md above the generated notes,
#                           marked pre-release exactly when the version has a pre-release part
#   one draft               replaces its assets: uploads every asset with --clobber, then deletes the
#                           assets of the draft that this run did not produce
#   a published release     fails and changes nothing
#   more than one draft     fails, naming them, and changes nothing (GitHub allows two drafts on one
#                           tag; this script never makes a second)
#
# The second form (`--check-assets`) only checks that the directory holds exactly the expected set
# for that version. packages.yml runs it on every package build, so a change that breaks the set (a
# new RPM subpackage, a debuginfo package) fails the pull request and not the first release.
#
# The releases are found through the API list and filtered on tag_name: a lookup by tag
# (`gh release view <tag>`) may not return a draft, and a draft is the case that matters on a re-run.
# Everything that can refuse is decided before the first write, here and on disk.
#
# Replacing a draft touches its assets only. For one tag the pre-release flag and the notes header
# cannot change (both come from the tag and from a file at the tagged commit), so only the notes
# GitHub generated at creation can go stale, and a refresh is not built.
#
# The tag's shape and whether it is a pre-release are version.sh's answers, not a pattern here.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
version_sh="$root/packaging/version.sh"
notes_source="$root/packaging/release-notes.md"

die() {
  echo "draft.sh: $*" >&2
  exit 1
}

# check_assets DIR UPSTREAM: DIR holds exactly 1 tarball, 1 source RPM, 3 RPMs, 3 debs and 1 Arch
# package, each named for UPSTREAM and none with "~". SHA256SUMS is not an asset. Sets `assets`.
#
# "Named for UPSTREAM" is exact. The tarball is proton-drive-sync-engine-<version>.tar.gz. The
# packages carry the version followed by their revision: -<version>-<number> followed by "." "_" or
# "-" (rpm -1.fc43., deb _..-1_, Arch -1-x86_64). A name for 0.1.0-beta.1 therefore does not pass
# for 0.1.0, and 0.1.0-beta.10 does not pass for 0.1.0-beta.1.
check_assets() {
  local dir="$1" upstream="$2" name class pattern escaped
  local tarballs=0 srpms=0 rpms=0 debs=0 arch=0
  [[ -d "$dir" ]] || die "$dir is not a directory"
  # A stale SHA256SUMS from an earlier run is not an asset.
  mapfile -t assets < <(find "$dir" -maxdepth 1 -mindepth 1 ! -name SHA256SUMS -printf '%f\n' | LC_ALL=C sort)
  ((${#assets[@]} > 0)) || die "$dir holds no assets"
  escaped="${upstream//./\\.}"
  pattern="[-_]${escaped}-[0-9]+[._-]"
  for name in "${assets[@]}"; do
    [[ -f "$dir/$name" ]] || die "$name is not a file"
    [[ "$name" != *"~"* ]] || die "the asset name $name contains '~'"
    case "$name" in
      proton-drive-sync-engine-*.tar.gz) class=tarball ;;
      *.src.rpm) class=srpm ;;
      *.rpm) class=rpm ;;
      *.deb) class=deb ;;
      *.pkg.tar.zst) class=arch ;;
      *) die "unexpected asset $name (the release holds the tarball, the packages and SHA256SUMS only)" ;;
    esac
    if [[ "$class" == tarball ]]; then
      [[ "$name" == "proton-drive-sync-engine-${upstream}.tar.gz" ]] ||
        die "the asset name $name is not named for version $upstream"
      tarballs=$((tarballs + 1))
    else
      [[ "$name" =~ $pattern ]] || die "the asset name $name is not named for version $upstream"
      case "$class" in
        srpm) srpms=$((srpms + 1)) ;;
        rpm) rpms=$((rpms + 1)) ;;
        deb) debs=$((debs + 1)) ;;
        arch) arch=$((arch + 1)) ;;
      esac
    fi
  done
  if ((tarballs != 1 || srpms != 1 || rpms != 3 || debs != 3 || arch != 1)); then
    die "expected 1 tarball, 1 source RPM, 3 RPMs, 3 debs and 1 Arch package; found $tarballs, $srpms, $rpms, $debs, $arch in $dir"
  fi
}

if [[ "${1:-}" == --check-assets ]]; then
  [[ $# -eq 3 ]] || die "usage: packaging/release/draft.sh --check-assets <assets-dir> <upstream version>"
  "$version_sh" is-prerelease "$3" >/dev/null || exit 1
  check_assets "$2" "$3"
  echo "draft.sh: ${#assets[@]} assets, all named for $3."
  exit 0
fi

[[ $# -eq 2 ]] || die "usage: packaging/release/draft.sh <tag> <assets-dir>"
tag="$1"
dir="$2"
repo="${GITHUB_REPOSITORY:-}"
[[ -n "$repo" ]] || die "GITHUB_REPOSITORY is not set"
[[ -d "$dir" ]] || die "$dir is not a directory"
[[ -s "$notes_source" ]] || die "$notes_source is missing or empty"

# The tag is validated before it is used for anything else (a jq filter, a gh argument).
upstream="$("$version_sh" tag-version "$tag")" || exit 1
prerelease="$("$version_sh" is-prerelease "$upstream")" || exit 1

# The pre-release warning sits between two marker lines in the notes source: exactly one start
# marker, then exactly one end marker. Anything else is refused for every tag, a final version
# included, because stripping a broken file would ship the warning or drop the install section.
awk '
  $0 == "<!-- pre-release -->"  { starts++; if (open) bad = 1; open = 1 }
  $0 == "<!-- /pre-release -->" { ends++; if (!open) bad = 1; open = 0 }
  END { exit !(starts == 1 && ends == 1 && !bad && !open) }
' "$notes_source" ||
  die "$notes_source must hold exactly one '<!-- pre-release -->' line followed by exactly one '<!-- /pre-release -->' line"

# --- 1. What exists for this tag. Nothing is written until this has been read. ------------------

pages="$(gh api "repos/$repo/releases" --paginate)" || die "could not list the releases of $repo"
# The API prints [] for a repository with no releases, so an empty answer is not "none": treating it
# as "none" would create a second draft.
[[ -n "${pages//[[:space:]]/}" ]] || die "the release list of $repo came back empty (expected at least [])"
# --paginate prints one JSON array per page.
releases="$(jq -s 'add // []' <<<"$pages")" || die "the release list was not JSON"
published="$(jq -r --arg tag "$tag" '.[] | select(.tag_name == $tag and (.draft | not)) | "\(.html_url)"' <<<"$releases")"
if [[ -n "$published" ]]; then
  die "the release for $tag is already published ($published): refusing to change it. Nothing was changed."
fi
drafts="$(jq -c --arg tag "$tag" '[.[] | select(.tag_name == $tag and .draft)]' <<<"$releases")"
draft_count="$(jq 'length' <<<"$drafts")"
if ((draft_count > 1)); then
  listing="$(jq -r '.[] | "  draft id \(.id) \(.html_url)"' <<<"$drafts")"
  die "there are $draft_count draft releases for $tag; delete all but one and run again. Nothing was changed.
$listing"
fi

# --- 2. The assets: exactly the expected set, named by the upstream version. -------------------

check_assets "$dir" "$upstream"

# --- 3. The checksums and the notes. -----------------------------------------------------------

(cd "$dir" && sha256sum -- "${assets[@]}" >SHA256SUMS)
files=("${assets[@]}" SHA256SUMS)
paths=()
for name in "${files[@]}"; do paths+=("$dir/$name"); done

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
notes="$work/notes.md"
# A pre-release keeps the warning (without the markers); a final release loses it.
if [[ "$prerelease" == true ]]; then
  sed -e '/^<!-- pre-release -->$/d' -e '/^<!-- \/pre-release -->$/d' "$notes_source" >"$notes"
else
  sed -e '/^<!-- pre-release -->$/,/^<!-- \/pre-release -->$/d' "$notes_source" >"$notes"
fi
[[ -s "$notes" ]] || die "the release notes are empty after the pre-release block was handled"

# --- 4. Write. ---------------------------------------------------------------------------------

if ((draft_count == 0)); then
  # --notes-file with --generate-notes: `gh release create --help` says notes given with --notes
  # are prepended to the generated ones; that the same holds for --notes-file is checked on the
  # first real draft (tasks.md 4.3).
  args=(release create "$tag" "${paths[@]}" --repo "$repo" --draft --verify-tag --generate-notes --notes-file "$notes")
  if [[ "$prerelease" == true ]]; then args+=(--prerelease); fi
  gh "${args[@]}"
  echo "draft.sh: created the draft release for $tag (pre-release: $prerelease)."
else
  # Upload first, then delete what this run did not produce, so the draft is never emptier than
  # the old one while it is being replaced.
  gh release upload "$tag" "${paths[@]}" --repo "$repo" --clobber
  mapfile -t existing < <(jq -r '.[0].assets[].name' <<<"$drafts")
  for name in "${existing[@]}"; do
    keep=false
    for new in "${files[@]}"; do
      if [[ "$name" == "$new" ]]; then keep=true; fi
    done
    if [[ "$keep" == false ]]; then
      echo "draft.sh: removing the stale asset $name"
      gh release delete-asset "$tag" "$name" --repo "$repo" --yes
    fi
  done
  echo "draft.sh: replaced the assets of the draft release for $tag."
fi
