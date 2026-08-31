#!/usr/bin/env bash
set -euo pipefail

if ! git rev-parse --verify HEAD >/dev/null 2>&1; then
  printf 'package verification requires a committed HEAD\n' >&2
  exit 1
fi
if ! git diff --quiet || ! git diff --cached --quiet; then
  printf 'package verification requires no tracked changes\n' >&2
  exit 1
fi

# Cargo 1.88 already ships the workspace-packaging implementation behind its
# feature gate. It creates a temporary local registry so unpublished workspace
# dependencies are verified in publish order.
RUSTC_BOOTSTRAP=1 cargo -Z package-workspace package --workspace --locked
