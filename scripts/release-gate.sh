#!/usr/bin/env bash
# The template benchmark gate (scripts/bench.py gate) on the committed HEAD,
# its verdict recorded on GitHub as the commit status `template-gate`. The
# Release workflow publishes only a commit whose `template-gate` status is
# `success` (#446).
#
# The gate is not a CI job: a template's run time over its reference's is
# judged to about 1%, and GitHub's runners differ from run to run by more
# than that (see docs/src/performance.md, "Where the gate runs"). Run this
# where nothing else builds; it takes the machine's build lock
# (AXIOVAL_GATE_LOCK, default ~/.cache/axioval/gate.lock) exclusively for
# the whole gate. With --no-status it only runs the gate.
set -euo pipefail

cd "$(dirname "$0")/.."

post=1
case "${1:-}" in
  "") ;;
  --no-status) post=0 ;;
  *)
    printf 'usage: scripts/release-gate.sh [--no-status]\n' >&2
    exit 2
    ;;
esac

if ! git diff --quiet || ! git diff --cached --quiet; then
  printf 'the release gate measures a committed HEAD: commit or stash tracked changes first\n' >&2
  exit 1
fi
sha="$(git rev-parse HEAD)"

lock="${AXIOVAL_GATE_LOCK:-$HOME/.cache/axioval/gate.lock}"
mkdir -p "$(dirname "$lock")"
out="target/bench/release-gate.jsonl"

# Built first, so the lock is held only while measuring.
cargo bench -p axioval-cli --bench templates --no-run

verdict=0
flock -x "$lock" python3 scripts/bench.py gate --out "$out" || verdict=$?

if [ "$post" = 1 ]; then
  if [ "$verdict" = 0 ]; then
    state=success description="every template within its budget"
  else
    state=failure description="a template over its budget or undecided; see scripts/bench.py"
  fi
  # GitHub knows only pushed commits: push HEAD before recording its verdict.
  if ! gh api -X POST "repos/{owner}/{repo}/statuses/$sha" \
    -f state="$state" -f context=template-gate -f description="$description" >/dev/null; then
    printf 'the gate gave %s, but its status could not be recorded on %s (pushed?)\n' "$state" "$sha" >&2
    exit 1
  fi
  printf 'template-gate: %s on %s\n' "$state" "$sha"
fi
exit "$verdict"
