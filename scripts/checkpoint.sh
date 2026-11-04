#!/usr/bin/env bash
# Usage: scripts/checkpoint.sh "<phase>" "<what was done>" "<next step>"
# Optional: CHECKPOINT_TRAILERS="Co-Authored-By: ..." appends trailer lines to the commit.
# Appends a PROGRESS row, commits everything, pushes with retries, verifies the remote.
set -euo pipefail
[ $# -eq 3 ] || { echo "usage: $0 <phase> <done> <next>" >&2; exit 2; }
branch=$(git rev-parse --abbrev-ref HEAD)
case "$branch" in main|master) echo "refusing to checkpoint on $branch" >&2; exit 1;; esac
progress="docs/v1/PROGRESS.md"
printf '| %s | %s | %s | %s |\n' "$(date -u '+%Y-%m-%d %H:%M')" "$1" "$2" "$3" >> "$progress"
git add -A
if [ -n "${CHECKPOINT_TRAILERS:-}" ]; then
  git commit -m "checkpoint($1): $2" -m "Next: $3" -m "$CHECKPOINT_TRAILERS"
else
  git commit -m "checkpoint($1): $2" -m "Next: $3"
fi
for wait in 2 4 8; do
  git push -u origin "$branch" && break
  echo "push failed, retrying in ${wait}s" >&2; sleep "$wait"
done
local_sha=$(git rev-parse HEAD)
remote_sha=$(git ls-remote origin "refs/heads/$branch" | cut -f1)
if [ "$local_sha" != "$remote_sha" ]; then
  echo "WARNING: remote does not match local ($remote_sha vs $local_sha); work is only saved locally" >&2
  exit 1
fi
echo "checkpoint saved: $local_sha"
