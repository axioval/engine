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
# feature gate. Its temporary local registry is not safe to reuse while
# unpublished workspace crates retain the same version, so every verification
# gets an isolated target directory.
package_target="$(mktemp -d "${TMPDIR:-/tmp}/axioval-package.XXXXXXXX")"
cleanup() {
  rm -rf -- "$package_target"
}
trap cleanup EXIT

CARGO_TARGET_DIR="$package_target" \
  RUSTC_BOOTSTRAP=1 \
  cargo -Z package-workspace package --workspace --locked
python3 scripts/check_package_contents.py "$package_target/package"
