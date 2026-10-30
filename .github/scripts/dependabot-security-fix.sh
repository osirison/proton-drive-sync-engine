#!/usr/bin/env bash
# Decides whether a Dependabot pull request is the fix for an open security alert, so that
# .github/workflows/dependabot-auto-merge.yml may arm auto-merge for a minor or major bump.
# Writes `fix=true` or `fix=false` to $GITHUB_OUTPUT and logs one line per alert considered
# (an alert whose number is missing or not a number is logged as `?`; only a wholly empty row,
# which is not an alert, is skipped).
#
# Why this is not dependabot/fetch-metadata's `alert-lookup`: that matches an alert by comparing
# the alert's `vulnerableRequirements` to "= <version>", which GitHub reports as a bare version,
# so it never matches and `ghsa-id` / `alert-state` stay empty (osirison/proton-drive-sync-engine#494).
# An "open alert exists" test alone is also too loose: an alert stays open for about an hour after
# its fix merges, so a later, ordinary minor bump of the same package would pass it.
#
# fix=true needs ALL of:
#   a. exactly one dependency (no comma in DEP_NAMES, no DEP_GROUP);
#   b. an alert that is open, for exactly this package, in exactly this manifest;
#   c. PREVIOUS is inside that alert's vulnerable_version_range;
#   d. the alert has a first_patched_version, NEW is at or above it, and NEW is in the same
#      compatibility line as it (see same_line);
#   e. NEW is strictly above PREVIOUS;
#   f. NEW is inside the vulnerable range of no open alert for this package and manifest. A
#      range this script cannot read counts as containing NEW, so such an alert blocks.
#
# Inputs (environment): GH_TOKEN, REPOSITORY (owner/name), ECOSYSTEM and DIRECTORY (from
# fetch-metadata's package-ecosystem and directory), DEP_NAMES, DEP_GROUP, PREVIOUS, NEW,
# GITHUB_OUTPUT.
#
# Versions are compared numerically only (digits and dots). Anything else, a prerelease or build
# metadata included, matches nothing. A lookup that fails (401, 403, network, unparseable
# reply) exits non-zero: it is never read as "no alerts".
set -euo pipefail

: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is not set}"

finish() {
  echo "fix=$1" >>"$GITHUB_OUTPUT"
  exit 0
}

no() {
  echo "Not a security fix: $1"
  finish false
}

# valid_version V: digits and dots only, components short enough for bash arithmetic.
valid_version() {
  [[ $1 =~ ^[0-9]+(\.[0-9]+)*$ ]] || return 1
  local IFS=. part
  for part in $1; do
    ((${#part} <= 15)) || return 1
  done
}

# vcmp A B: sets CMP to -1, 0 or 1. A missing component is 0, so 1.2 equals 1.2.0.
vcmp() {
  local -a x y
  local IFS=.
  read -ra x <<<"$1"
  read -ra y <<<"$2"
  local n=${#x[@]} i a b
  if ((${#y[@]} > n)); then n=${#y[@]}; fi
  for ((i = 0; i < n; i++)); do
    a=$((10#${x[i]:-0}))
    b=$((10#${y[i]:-0}))
    if ((a < b)); then
      CMP=-1
      return 0
    fi
    if ((a > b)); then
      CMP=1
      return 0
    fi
  done
  CMP=0
}

# same_line V PATCHED: V shares the compatibility line of PATCHED, meaning the leading components
# of PATCHED up to and including its first non-zero one are equal in V (a missing component is 0).
# 1.2.5 and 1.9.0 share a line; 1.2.5 and 3.0.0, 0.3.2 and 0.5.0, 0.0.3 and 0.0.4 do not.
# A bump that crosses into another line is not the fix the alert asked for.
same_line() {
  local -a x y
  local IFS=.
  read -ra x <<<"$1"
  read -ra y <<<"$2"
  local n=${#x[@]} i a b
  if ((${#y[@]} > n)); then n=${#y[@]}; fi
  for ((i = 0; i < n; i++)); do
    a=$((10#${x[i]:-0}))
    b=$((10#${y[i]:-0}))
    ((a == b)) || return 1
    ((b == 0)) || return 0
  done
  return 0
}

# in_range V RANGE: RANGE is comma-separated `op version`, op one of = < <= > >=. Every
# comparator must hold. Returns 0 when V is inside, 1 when it is not, 2 when the range is not
# one this script reads.
in_range() {
  local version=$1 range=$2 part op bound i
  local -a parts ops bounds
  [[ $range =~ ^[^,]+(,[^,]+)*$ ]] || return 2
  IFS=',' read -ra parts <<<"$range"
  # Every comparator is read before any is applied, so an unreadable one is never hidden by an
  # earlier one that already failed.
  for part in "${parts[@]}"; do
    part=${part#"${part%%[![:space:]]*}"}
    part=${part%"${part##*[![:space:]]}"}
    [[ $part =~ ^(=|<=|>=|<|>)[[:space:]]*([0-9.]+)$ ]] || return 2
    op=${BASH_REMATCH[1]}
    bound=${BASH_REMATCH[2]}
    valid_version "$bound" || return 2
    ops+=("$op")
    bounds+=("$bound")
  done
  for i in "${!ops[@]}"; do
    vcmp "$version" "${bounds[i]}"
    case ${ops[i]} in
      '=') ((CMP == 0)) || return 1 ;;
      '<') ((CMP < 0)) || return 1 ;;
      '<=') ((CMP <= 0)) || return 1 ;;
      '>') ((CMP > 0)) || return 1 ;;
      '>=') ((CMP >= 0)) || return 1 ;;
    esac
  done
}

# a. exactly one dependency.
if [[ -n ${DEP_GROUP:-} ]]; then
  no "the pull request is in a dependency group, which carries unrelated bumps"
fi
if [[ -z ${DEP_NAMES:-} || $DEP_NAMES == *,* ]]; then
  no "the pull request does not update exactly one dependency"
fi

case ${ECOSYSTEM:-} in
  npm_and_yarn) lockfile=package-lock.json ;;
  cargo) lockfile=Cargo.lock ;;
  *) no "ecosystem '${ECOSYSTEM:-}' has no manifest mapping" ;;
esac

directory=${DIRECTORY:-}
if [[ ! $directory =~ ^/[A-Za-z0-9._/-]*$ || $directory == *..* || $directory == *//* ]]; then
  no "directory is not a plain absolute path"
fi
directory=${directory#/}
directory=${directory%/}
if [[ -z $directory ]]; then
  manifest=$lockfile
else
  manifest=$directory/$lockfile
fi

if [[ ! $DEP_NAMES =~ ^[@A-Za-z0-9._/-]+$ ]]; then
  no "the package name has characters outside the allowed set"
fi
if ! valid_version "${PREVIOUS:-}" || ! valid_version "${NEW:-}"; then
  no "previous version '${PREVIOUS:-}' or new version '${NEW:-}' is not a plain numeric version"
fi
vcmp "$NEW" "$PREVIOUS"
if ((CMP <= 0)); then
  no "new version $NEW is not above the previous version $PREVIOUS"
fi
if [[ -z ${GH_TOKEN:-} ]]; then
  no "no DEPENDABOT_ALERTS_TOKEN is set, so the alerts cannot be read"
fi
if [[ ! ${REPOSITORY:-} =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]; then
  echo "REPOSITORY is not owner/name" >&2
  exit 1
fi

package_query=$(jq -rn --arg v "$DEP_NAMES" '$v | @uri')
manifest_query=$(jq -rn --arg v "$manifest" '$v | @uri')
endpoint="repos/$REPOSITORY/dependabot/alerts?state=open&package=$package_query&manifest=$manifest_query&per_page=100"

# b. The server filters; every alert is checked again below, and the filter is not trusted.
if ! reply=$(gh api --paginate "$endpoint"); then
  echo "The alert lookup failed; refusing to treat that as 'no alerts'." >&2
  exit 1
fi
if [[ -z $reply ]]; then
  echo "The alert lookup returned nothing." >&2
  exit 1
fi
# --paginate prints one JSON array per page. Fields are joined with a unit separator, which
# `read` does not collapse the way it collapses runs of tabs.
if ! rows=$(
  jq -r -s --arg name "$DEP_NAMES" --arg manifest "$manifest" '
    if all(.[]; type == "array") then add // [] else error("not a list of alerts") end
    | .[]
    | select(type == "object")
    | [
        (.number | tostring),
        (if .state != "open" then "state is not open"
         elif (.dependency.package.name // "") != $name
              or (.security_vulnerability.package.name // $name) != $name
           then "package is not this one"
         elif (.dependency.manifest_path // "") != $manifest then "manifest is not this one"
         else "ok" end),
        (.security_vulnerability.vulnerable_version_range // ""),
        (.security_vulnerability.first_patched_version.identifier // "")
      ]
    | map(gsub("[[:cntrl:]]"; " "))
    | join("\u001f")
  ' <<<"$reply"
); then
  echo "The alert lookup returned something that is not a list of alerts." >&2
  exit 1
fi

match=false
blocked=false
while IFS=$'\x1f' read -r number verdict range patched; do
  # Only a wholly empty row (jq prints nothing when there are no alerts) is skipped. An alert with
  # a missing or odd number is still judged, and logged as `?`.
  [[ -n $number$verdict$range$patched ]] || continue
  [[ $number =~ ^[0-9]+$ ]] || number='?'
  if [[ $verdict != ok ]]; then
    echo "alert $number: no match, $verdict"
    continue
  fi
  # f. Any open alert of this package and manifest whose range holds NEW, or whose range cannot
  # be read, means this update does not clear them all. 1 is the only answer that clears NEW.
  inside=0
  in_range "$NEW" "$range" || inside=$?
  if ((inside != 1)); then
    echo "alert $number: blocks, new version $NEW is inside the vulnerable range '$range' or the range is not one this script reads"
    blocked=true
    continue
  fi
  # c.
  if ! in_range "$PREVIOUS" "$range"; then
    echo "alert $number: no match, previous version $PREVIOUS is not inside the vulnerable range '$range'"
    continue
  fi
  # d.
  if ! valid_version "$patched"; then
    echo "alert $number: no match, no usable first patched version '$patched'"
    continue
  fi
  vcmp "$NEW" "$patched"
  if ((CMP < 0)); then
    echo "alert $number: no match, new version $NEW is below the patched version $patched"
    continue
  fi
  # d, continued.
  if ! same_line "$NEW" "$patched"; then
    echo "alert $number: no match, new version $NEW is not in the compatibility line of the patched version $patched"
    continue
  fi
  echo "alert $number: match, $DEP_NAMES $PREVIOUS is inside '$range' and $NEW is at or above the patched $patched, in $manifest"
  match=true
done <<<"$rows"

if [[ $match == true && $blocked == true ]]; then
  no "an open alert for $DEP_NAMES in $manifest still covers $NEW or has a range this script cannot read"
fi
if [[ $match == true ]]; then
  finish true
fi
no "no open alert for $DEP_NAMES in $manifest that this update fixes"
