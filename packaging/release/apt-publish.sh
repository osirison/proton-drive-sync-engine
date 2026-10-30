#!/usr/bin/env bash
# The publish job's apt step (openspec change add-linux-release-pipeline, task 6.3, design D10).
#
#   packaging/release/apt-publish.sh <tag>                                   the channel step
#   packaging/release/apt-publish.sh --local <out-dir> <upstream>=<dir>...    a tree on disk, for the
#                                                                            pull-request job and tests
#
# THE TREE IS A FUNCTION OF THE PUBLISHED RELEASES. Every run rebuilds both suites from the releases
# GitHub lists as published (the releases are the store; the apt repository keeps nothing that cannot
# be rebuilt), whatever PUBLISH_CHANNELS says. That is how "a pre-release goes to beta only, a final
# release to beta and stable" comes out: a pre-release can only enter the beta suite, a final release
# enters both, and a run for one never drops or changes the other suite. PUBLISH_CHANNELS is required
# (it proves route.sh ran) and logged; it does not select anything.
#
# WHICH RELEASES COUNT (the "newest three" rule). Releases are ordered by VERSION, never by date:
# `version.sh sort-key` (major, minor, patch as numbers; then alpha < beta < rc < the final release;
# then the pre-release number as a number), the order dpkg gives the packages' own versions. Of the
# published releases whose tag is a release tag:
#   suite beta     the newest 3 of ALL of them, pre-releases and finals (design D9: a final release
#                  reaches the beta channel too)
#   suite stable   the newest 3 FINAL ones; the suite does not exist until one is published
# A release is left out if its tag is listed in APT_WITHDRAWN (space or comma separated; the
# repository variable of that name; this is how a version is withdrawn, and it works whether or not
# the releases are immutable). A published release whose pre-release flag disagrees with its tag
# stops the run when it would be used: a beta marked final would otherwise reach stable.
#
# WHAT IT DOES
#   1. downloads the `.deb` assets and SHA256SUMS of every release it uses; every deb must be listed
#      in that release's SHA256SUMS with the same SHA-256, and a release in the window with no deb, a
#      deb the sums do not list or a mismatch stops the step (nothing is skipped silently);
#   2. puts them in pool/main/p/proton-drive-sync-engine/ under their asset names and writes each
#      suite's `Packages` with apt-ftparchive, from a sorted file list, so the same releases give
#      the same bytes (the only text that changes between runs is `Date:` in Release, and the
#      signatures);
#   3. writes `Release`, signs `InRelease` (clearsigned) and `Release.gpg` with APT_SIGNING_KEY_FILE
#      in a GNUPGHOME that is always removed, and publishes the public key as
#      proton-drive-sync-engine.gpg and .asc. A key file may hold several secret keys (a rotation);
#      all of them sign. In a real run their fingerprints must be exactly the ones recorded in
#      apt-key-fingerprints.txt;
#   4. clones gh-pages of the apt repository with the deploy key, compares, and force-pushes a
#      single-commit gh-pages only if the deterministic files differ (see trees_equal).
#
# A DRY RUN does every read and writes nothing: the downloads, the tree, the clone and the
# comparison, and the signing key is used for ONE test signature (made in the throwaway home, over
# the real Release, verified and discarded); no InRelease is signed into the tree and nothing is
# pushed.
#
# Needs gh, jq, git, gpg, gpgv, apt-ftparchive (apt-utils), GITHUB_REPOSITORY, PUBLISH_CHANNELS,
# PUBLISH_DRY_RUN, APT_SIGNING_KEY_FILE, APT_DEPLOY_KEY_FILE, APT_GIT_NAME, APT_GIT_EMAIL;
# optional APT_WITHDRAWN, APT_FINGERPRINT_FILE (a test seam), APT_REMOTE_URL (replaces the apt
# repository's URL; the tests use a local bare repository), PUBLISH_GPG_TMPDIR (where the throwaway
# home is made; the tests look there afterwards).
#
# Known limit: two different tags published at the same moment can race on the force-push (the
# workflow's concurrency group is per tag). The second one rebuilds from the same releases, so a
# re-run of either repairs it.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

readonly KEEP=3
readonly POOL="pool/main/p/${PACKAGE_BASE}"
readonly KEY_BASENAME="${PACKAGE_BASE}"

work=""
gnupg_home=""

cleanup() {
  if [[ -n "$gnupg_home" ]]; then
    GNUPGHOME="$gnupg_home" gpgconf --kill gpg-agent >/dev/null 2>&1 || true
    rm -rf -- "$gnupg_home"
  fi
  if [[ -n "$work" ]]; then
    rm -rf -- "$work"
  fi
}
trap cleanup EXIT

log() { echo "apt-publish.sh: $*" >&2; }

# --- which releases ---------------------------------------------------------------------------------

# newest MODE: reads upstream versions, one per line, and prints the newest $KEEP, newest first.
# MODE `all` keeps pre-releases, `final` drops them.
newest() {
  local mode="$1" upstream
  while IFS= read -r upstream; do
    [[ -n "$upstream" ]] || continue
    if [[ "$mode" == final && "$("$version_sh" is-prerelease "$upstream")" == true ]]; then
      continue
    fi
    printf '%s %s\n' "$("$version_sh" sort-key "$upstream")" "$upstream"
  done | LC_ALL=C sort -r | awk -v n="$KEEP" 'NR <= n { print $2 }'
}

# --- a release's assets -------------------------------------------------------------------------------

# check_assets DIR UPSTREAM: every .deb in DIR is a plain file named for UPSTREAM and is listed in
# DIR/SHA256SUMS with its own SHA-256. Prints the deb names, sorted.
check_assets() {
  local dir="$1" upstream="$2" escaped name listed actual count=0 main=0
  [[ -s "$dir/SHA256SUMS" ]] || { log "$upstream: the release has no SHA256SUMS"; return 1; }
  escaped="${upstream//./\\.}"
  local names
  names="$(find "$dir" -maxdepth 1 -mindepth 1 -name '*.deb' -printf '%f\n' | LC_ALL=C sort)" || return 1
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    # The names come from the release API: no path, and the shape draft.sh gave them.
    if [[ ! "$name" =~ ^[a-z0-9][a-z0-9.+-]*_${escaped}-[0-9]+_(amd64|all)\.deb$ ]]; then
      log "$upstream: '$name' is not a deb named for this version"
      return 1
    fi
    if [[ -L "$dir/$name" || ! -f "$dir/$name" ]]; then
      log "$upstream: $name is not a plain file"
      return 1
    fi
    listed="$(awk -v n="$name" '$2 == n || $2 == "*" n { print $1 }' "$dir/SHA256SUMS")"
    actual="$(sha256sum "$dir/$name" | awk '{print $1}')"
    if [[ -z "$listed" || "$listed" == *$'\n'* ]]; then
      log "$upstream: SHA256SUMS lists $name ${listed:+more than once}${listed:-not at all}"
      return 1
    fi
    if [[ "$listed" != "$actual" ]]; then
      log "$upstream: SHA256SUMS lists $listed for $name but the file's SHA-256 is $actual"
      return 1
    fi
    count=$((count + 1))
    if [[ "$name" == "${PACKAGE_BASE}_"*_amd64.deb ]]; then main=$((main + 1)); fi
  done <<<"$names"
  ((count > 0)) || { log "$upstream: the release holds no .deb"; return 1; }
  ((main == 1)) || { log "$upstream: the release must hold exactly one ${PACKAGE_BASE}_*_amd64.deb"; return 1; }
  printf '%s\n' "$names"
}

# stage_release UPSTREAM DIR: check the release's debs and copy them into the pool of $tree. Records
# their pool paths in debs_of[UPSTREAM].
declare -A debs_of=()
stage_release() {
  local upstream="$1" dir="$2" names name
  names="$(check_assets "$dir" "$upstream")" || die "release $upstream is not usable"
  mkdir -p "$tree/$POOL"
  while IFS= read -r name; do
    cp -p -- "$dir/$name" "$tree/$POOL/$name"
    debs_of[$upstream]+="$POOL/$name"$'\n'
  done <<<"$names"
}

# --- the tree -----------------------------------------------------------------------------------------

# generate_tree: Packages and Release for each suite in `suites`, from debs_of and the suite's
# releases (suite_releases_<suite>).
generate_tree() {
  local suite upstream conf="$work/ftparchive.conf"
  mkdir -p "$work/lists" "$work/cache"
  {
    printf 'Dir { ArchiveDir "%s"; CacheDir "%s"; };\n' "$tree" "$work/cache"
    printf 'Default { Packages::Compress "."; };\n'
  } >"$conf"
  for suite in "${suites[@]}"; do
    local reference="suite_releases_${suite}[@]"
    mkdir -p "$tree/dists/$suite/main/binary-amd64"
    # One sorted list per suite: the order of Packages is the order of this file, not the order the
    # files were made in or the API listed the releases in.
    for upstream in "${!reference}"; do
      printf '%s' "${debs_of[$upstream]}"
    done | LC_ALL=C sort -u >"$work/lists/$suite"
    printf 'BinDirectory "dists/%s/main/binary-amd64" { Packages "dists/%s/main/binary-amd64/Packages"; FileList "%s"; };\n' \
      "$suite" "$suite" "$work/lists/$suite" >>"$conf"
  done
  apt-ftparchive generate "$conf" >/dev/null || die "apt-ftparchive could not write the Packages files"
  for suite in "${suites[@]}"; do
    [[ -s "$tree/dists/$suite/main/binary-amd64/Packages" ]] || die "the $suite suite has an empty Packages file"
    apt-ftparchive \
      -o "APT::FTPArchive::Release::Origin=${PACKAGE_BASE}" \
      -o "APT::FTPArchive::Release::Label=${PACKAGE_BASE}" \
      -o "APT::FTPArchive::Release::Suite=$suite" \
      -o "APT::FTPArchive::Release::Codename=$suite" \
      -o "APT::FTPArchive::Release::Architectures=amd64" \
      -o "APT::FTPArchive::Release::Components=main" \
      -o "APT::FTPArchive::Release::Description=Proton Drive sync engine ($suite)" \
      release "$tree/dists/$suite" >"$work/Release.$suite" || die "apt-ftparchive could not write the Release file of $suite"
    mv -f -- "$work/Release.$suite" "$tree/dists/$suite/Release"
  done
  : >"$tree/.nojekyll"
}

# --- signing ------------------------------------------------------------------------------------------

# gpg_in_home ARGS: gpg in the throwaway home, no prompts, no passphrase.
gpg_in_home() {
  GNUPGHOME="$gnupg_home" gpg --batch --yes --quiet --pinentry-mode loopback --passphrase '' "$@"
}

# open_signing_key: imports APT_SIGNING_KEY_FILE into a new throwaway home and sets `fingerprints`
# (the primary keys of the file, sorted). The home is short because gpg-agent's socket path has a
# length limit.
open_signing_key() {
  [[ -n "${APT_SIGNING_KEY_FILE-}" && -s "$APT_SIGNING_KEY_FILE" ]] ||
    die "APT_SIGNING_KEY_FILE is not a readable file: the secret APT_SIGNING_KEY is missing"
  gnupg_home="$(mktemp -d "${PUBLISH_GPG_TMPDIR:-/tmp}/pdse-gpg.XXXXXX")"
  chmod 700 "$gnupg_home"
  gpg_in_home --import "$APT_SIGNING_KEY_FILE" 2>/dev/null || die "gpg could not import the signing key"
  mapfile -t fingerprints < <(GNUPGHOME="$gnupg_home" gpg --batch --with-colons --list-secret-keys |
    awk -F: '$1 == "sec" { want = 1; next } $1 == "fpr" && want { print $10; want = 0 }' | LC_ALL=C sort -u)
  ((${#fingerprints[@]} > 0)) || die "the signing key file holds no secret key"
  log "signing keys: ${fingerprints[*]}"
}

# A real run signs only with the keys the repository's docs and verification know.
require_recorded_fingerprints() {
  local recorded actual
  recorded="$("$publish_here/apt-client.sh" fingerprints)" || exit 1
  actual="$(printf '%s\n' "${fingerprints[@]}")"
  [[ "$recorded" == "$actual" ]] ||
    die "the signing key file holds the keys '${actual//$'\n'/ }' but apt-key-fingerprints.txt records '${recorded//$'\n'/ }': nothing is signed (docs/releasing.md, key rotation)"
}

# sign_files IN OUT_CLEARSIGNED OUT_DETACHED: both signatures, over SHA-512, by every key.
sign_release() {
  local release="$1" clear="$2" detached="${3-}" fingerprint users=()
  for fingerprint in "${fingerprints[@]}"; do
    users+=(--local-user "$fingerprint")
  done
  gpg_in_home --digest-algo SHA512 "${users[@]}" --clearsign --output "$clear" "$release" ||
    die "the signing key cannot sign (a passphrase, or an expired or revoked key)"
  if [[ -n "$detached" ]]; then
    gpg_in_home --digest-algo SHA512 "${users[@]}" --armor --detach-sign --output "$detached" "$release" ||
      die "the signing key cannot make a detached signature"
  fi
}

# write_keys: the public keys, binary (.gpg, what Signed-By names) and armored (.asc).
write_keys() {
  GNUPGHOME="$gnupg_home" gpg --batch --export "${fingerprints[@]}" >"$tree/$KEY_BASENAME.gpg"
  GNUPGHOME="$gnupg_home" gpg --batch --armor --export "${fingerprints[@]}" >"$tree/$KEY_BASENAME.asc"
  [[ -s "$tree/$KEY_BASENAME.gpg" && -s "$tree/$KEY_BASENAME.asc" ]] || die "gpg exported no public key"
}

# sign_tree: InRelease and Release.gpg for every suite, and the keys. Then what a client does: verify
# both signatures against the published key file with gpgv.
sign_tree() {
  local suite
  write_keys
  for suite in "${suites[@]}"; do
    sign_release "$tree/dists/$suite/Release" "$tree/dists/$suite/InRelease" "$tree/dists/$suite/Release.gpg"
    gpgv --keyring "$tree/$KEY_BASENAME.gpg" "$tree/dists/$suite/InRelease" >/dev/null 2>&1 ||
      die "gpgv does not accept $suite/InRelease with the published key"
    gpgv --keyring "$tree/$KEY_BASENAME.gpg" "$tree/dists/$suite/Release.gpg" "$tree/dists/$suite/Release" >/dev/null 2>&1 ||
      die "gpgv does not accept $suite/Release.gpg with the published key"
  done
}

# A dry run's one use of the key: sign the real Release of the first suite into the throwaway home,
# check it against the key's own public half, and leave nothing behind in the tree.
test_sign() {
  local suite="${suites[0]}"
  local public="$gnupg_home/test-key.gpg"
  GNUPGHOME="$gnupg_home" gpg --batch --export "${fingerprints[@]}" >"$public"
  sign_release "$tree/dists/$suite/Release" "$gnupg_home/test.InRelease"
  gpgv --keyring "$public" "$gnupg_home/test.InRelease" >/dev/null 2>&1 ||
    die "the test signature does not verify"
  rm -f -- "$gnupg_home/test.InRelease" "$public"
  log "the signing key made a test signature over $suite/Release (verified and discarded)"
}

# --- comparing and pushing ----------------------------------------------------------------------------

# trees_equal NEW OLD: the same pool, Packages, keys and .nojekyll, and Release files that are equal
# but for their Date line. InRelease and Release.gpg differ on every run (a signature carries its
# time) and Release's Date is the clock, so none of them can decide "changed"; everything they
# state is derived from the files compared here.
trees_equal() {
  local new="$1" old="$2" releases_new releases_old release
  diff -r -q -x .git -x InRelease -x Release.gpg -x Release "$new" "$old" >/dev/null 2>&1 || return 1
  releases_new="$(cd "$new" && find dists -name Release | LC_ALL=C sort)"
  releases_old="$(cd "$old" && find dists -name Release | LC_ALL=C sort)"
  [[ "$releases_new" == "$releases_old" ]] || return 1
  while IFS= read -r release; do
    [[ -n "$release" ]] || continue
    [[ "$(grep -v '^Date:' "$new/$release")" == "$(grep -v '^Date:' "$old/$release")" ]] || return 1
  done <<<"$releases_new"
}

publish_tree() {
  local url remote="$work/remote" push="$work/push" heads
  [[ -n "${APT_DEPLOY_KEY_FILE-}" && -s "$APT_DEPLOY_KEY_FILE" ]] ||
    die "APT_DEPLOY_KEY_FILE is not a readable file: the secret APT_DEPLOY_KEY is missing"
  [[ -n "${APT_GIT_NAME-}" && -n "${APT_GIT_EMAIL-}" ]] ||
    die "APT_GIT_NAME and APT_GIT_EMAIL are not set: the commit needs an identity"
  url="$(apt_repository_url)"
  export GIT_SSH_COMMAND
  GIT_SSH_COMMAND="$(ssh_command_for "$APT_DEPLOY_KEY_FILE")"
  export GIT_TERMINAL_PROMPT=0
  heads="$(git ls-remote --heads "$url" gh-pages)" || die "cannot read $url with the deploy key"
  if [[ -n "$heads" ]]; then
    git clone --quiet --single-branch --branch gh-pages "$url" "$remote" || die "cannot clone gh-pages of $url"
    if trees_equal "$tree" "$remote"; then
      log "gh-pages already holds this tree (the files that decide it are equal): no push"
      return 0
    fi
  else
    log "$url has no gh-pages branch yet"
  fi
  if [[ "$PUBLISH_DRY_RUN" == true ]]; then
    log "DRY RUN: gh-pages of $url differs from this tree: would force-push a single commit"
    return 0
  fi
  mkdir -p "$push"
  cp -a -- "$tree/." "$push/"
  git -C "$push" init --quiet
  git -C "$push" symbolic-ref HEAD refs/heads/gh-pages
  git -C "$push" add -A
  git -C "$push" -c "user.name=$APT_GIT_NAME" -c "user.email=$APT_GIT_EMAIL" -c commit.gpgsign=false \
    commit --quiet -m "Publish ${tag_upstream_value}"
  git -C "$push" push --quiet --force "$url" HEAD:refs/heads/gh-pages || die "cannot push gh-pages to $url"
  log "pushed gh-pages ($(git -C "$push" rev-parse --short HEAD)) to $url"
}

# --- the two ways in ------------------------------------------------------------------------------------

# choose_suites: from the list of usable upstream versions on stdin, set suites and suite_releases_*.
declare -a suite_releases_beta=() suite_releases_stable=() suites=()
choose_suites() {
  local all
  all="$(cat)"
  mapfile -t suite_releases_beta < <(newest all <<<"$all")
  mapfile -t suite_releases_stable < <(newest final <<<"$all")
  ((${#suite_releases_beta[@]} > 0)) || die "there is no published release to build the repository from"
  suites=(beta)
  ((${#suite_releases_stable[@]} == 0)) || suites+=(stable)
  log "suite beta: ${suite_releases_beta[*]}"
  if ((${#suite_releases_stable[@]} > 0)); then
    log "suite stable: ${suite_releases_stable[*]}"
  else
    log "suite stable: not created (no final release is published)"
  fi
}

union_of_suites() {
  printf '%s\n' "${suite_releases_beta[@]}" "${suite_releases_stable[@]}" | LC_ALL=C sort -u | sed '/^$/d'
}

run_local() {
  [[ $# -ge 2 ]] || die "usage: apt-publish.sh --local <out-dir> <upstream>=<dir>..."
  local out="$1" spec upstream dir
  shift
  work="$(mktemp -d)"
  tree="$work/tree"
  mkdir -p "$tree"
  declare -A dir_of=()
  for spec in "$@"; do
    upstream="${spec%%=*}"
    dir="${spec#*=}"
    [[ "$spec" == *=* ]] || die "'$spec' is not <upstream>=<dir>"
    "$version_sh" is-prerelease "$upstream" >/dev/null || exit 1
    dir_of[$upstream]="$dir"
  done
  choose_suites < <(printf '%s\n' "${!dir_of[@]}")
  local release
  while IFS= read -r release; do
    stage_release "$release" "${dir_of[$release]}"
  done < <(union_of_suites)
  generate_tree
  open_signing_key
  sign_tree
  mkdir -p "$out"
  cp -a -- "$tree/." "$out/"
  log "wrote the tree to $out (a local run: nothing was downloaded or pushed)"
}

run_publish() {
  tag="$1"
  record_on_exit apt
  require_dry_run
  require_channels
  tag_upstream_value="$(tag_upstream "$tag")" || exit 1
  [[ -n "${GITHUB_REPOSITORY-}" ]] || die "GITHUB_REPOSITORY is not set"
  log "$tag is routed to: ${channels[*]}; the tree is rebuilt from every published release"
  work="$(mktemp -d)"
  tree="$work/tree"
  mkdir -p "$tree"

  # 1. The published releases, minus the withdrawn ones.
  local -A withdrawn=() flag_of=()
  local entry rows upstream flag
  # APT_WITHDRAWN is optional (unset or empty withdraws nothing). Its names are split on commas and
  # white space with `tr`, never by the shell's own word splitting, which would expand a `v*`
  # against the working directory.
  local -a withdrawn_tags=()
  mapfile -t withdrawn_tags < <(printf '%s' "${APT_WITHDRAWN-}" | tr ',[:space:]' '\n\n' | sed '/^$/d')
  for entry in ${withdrawn_tags[@]+"${withdrawn_tags[@]}"}; do
    upstream="$("$version_sh" tag-version "$entry")" || die "APT_WITHDRAWN names '$entry', which is not a release tag"
    withdrawn[$upstream]=1
  done
  rows="$(gh api "repos/${GITHUB_REPOSITORY}/releases?per_page=100" --paginate |
    jq -r '.[] | select(.draft == false) | [.tag_name, (.prerelease | tostring)] | @tsv')" ||
    die "cannot list the releases of ${GITHUB_REPOSITORY}"
  local candidates=() row_tag
  while IFS=$'\t' read -r row_tag flag; do
    [[ -n "$row_tag" ]] || continue
    if ! upstream="$("$version_sh" tag-version "$row_tag" 2>/dev/null)"; then
      log "ignoring the release $row_tag: its tag is not a release tag"
      continue
    fi
    if [[ -n "${withdrawn[$upstream]-}" ]]; then
      log "leaving out $row_tag: it is in APT_WITHDRAWN"
      continue
    fi
    flag_of[$upstream]="$flag"
    candidates+=("$upstream")
  done <<<"$rows"
  choose_suites < <(printf '%s\n' "${candidates[@]}")

  # 2. Download and check what is used.
  local release expected
  while IFS= read -r release; do
    expected="$("$version_sh" is-prerelease "$release")"
    [[ "${flag_of[$release]}" == "$expected" ]] ||
      die "the release v$release is marked prerelease=${flag_of[$release]} but its version says $expected: fix the flag on the release; nothing is published"
    mkdir -p "$work/assets/$release"
    gh release download "v$release" --repo "$GITHUB_REPOSITORY" --dir "$work/assets/$release" \
      --pattern '*.deb' --pattern SHA256SUMS || die "cannot download the .deb assets and SHA256SUMS of v$release"
    stage_release "$release" "$work/assets/$release"
  done < <(union_of_suites)

  # 3. The tree, signed (or test-signed in a dry run).
  generate_tree
  open_signing_key
  require_recorded_fingerprints
  if [[ "$PUBLISH_DRY_RUN" == true ]]; then
    # The public keys are part of what gh-pages holds and the comparison reads them; writing them
    # signs nothing. Without them a dry run after a real publish of the same releases would report a
    # difference every time.
    write_keys
    test_sign
  else
    sign_tree
  fi

  # 4. Compare with gh-pages and push.
  publish_tree
  finish_recording
}

tree=""
tag=""
tag_upstream_value=""
case "${1-}" in
  --local)
    shift
    run_local "$@"
    ;;
  "" | -*)
    die "usage: apt-publish.sh <tag> | apt-publish.sh --local <out-dir> <upstream>=<dir>..."
    ;;
  *)
    [[ $# -eq 1 ]] || die "usage: apt-publish.sh <tag>"
    run_publish "$1"
    ;;
esac
