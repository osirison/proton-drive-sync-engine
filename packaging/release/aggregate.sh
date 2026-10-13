#!/usr/bin/env bash
# The publish job's last step (openspec change add-linux-release-pipeline, task 6.5): the job fails
# if any step did not end well, and says which.
#
#   packaging/release/aggregate.sh <step>...        e.g. preflight aur copr
#
# Each channel step runs even when an earlier one failed, so the job's own status would be decided by
# whichever step failed first and the others would go unnamed. Every step records `ok` or `failed`
# in the results directory (publish-lib.sh record_on_exit); a step with no record at all did not
# run, or was killed, and counts as failed. Exit 1 names every such step. A failed `preflight` is
# reported first, because it is why the channel steps did not run.
set -euo pipefail
# shellcheck source=packaging/release/publish-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/publish-lib.sh"

[[ $# -ge 1 ]] || die "usage: aggregate.sh <step>..."
dir="$(results_dir)"
bad=()
for step in "$@"; do
  result="missing (the step did not run or was killed)"
  if [[ -f "$dir/$step" ]]; then
    result="$(head -n1 "$dir/$step")"
  fi
  if [[ "$result" == ok ]]; then
    echo "aggregate.sh: $step: ok" >&2
  else
    echo "aggregate.sh: $step: $result" >&2
    bad+=("$step")
  fi
done

if ((${#bad[@]} > 0)); then
  die "these steps did not succeed: ${bad[*]}. Re-run the channel with workflow_dispatch once the cause is fixed (each channel step is safe to repeat)"
fi
echo "aggregate.sh: every step succeeded." >&2
