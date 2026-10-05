#!/usr/bin/env bash
# Repository gate.
#
# Usage: scripts/check.sh [section...]
#
# With no argument every section runs, in order: this is the full gate. CI
# runs each section as its own parallel job and passes only when all of them
# pass, so the union is identical:
#
#   deny   dependency policy (cargo-deny)
#   lint   architecture and packaging self-tests, formatting, clippy
#   test   workspace tests, the IDS corpus when IDS_TEST_CASES is set, the
#          public parity models when AXIOVAL_PARITY_MODELS names their
#          cache (scripts/parity_models.py fetch), and private parity cases
#          when AXIOVAL_PARITY_CASES is set
#   docs   rustdoc and the mdBook
set -euo pipefail

export RUSTFLAGS=""

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'required tool not found: %s\n' "$1" >&2
    exit 1
  fi
}

check_deny() {
  if [[ "${AXIOVAL_DEPENDENCY_AUDIT_COMPLETE:-0}" != "1" ]]; then
    require cargo-deny
    cargo deny check
  fi
}

check_lint() {
  python3 scripts/architecture.py --self-test
  python3 scripts/staging_isolation.py
  python3 scripts/migration.py
  python3 scripts/test_check_package_contents.py
  python3 scripts/test_publish_workspace.py
  python3 scripts/test_not_evaluated_inventory.py
  python3 scripts/test_parity_models.py
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
}

check_test() {
  cargo test --workspace --all-features
  # The buildingSMART IDS corpus is CC BY-ND 4.0 and is not vendored, so its
  # conformance test is ignored by default and runs only where a maintainer
  # points IDS_TEST_CASES at a checkout of it.
  if [[ -n "${IDS_TEST_CASES:-}" ]]; then
    cargo test -p axioval-ids -- --ignored corpus
  fi
  # Parity of re-expressions on the pinned public models, which are not
  # vendored: CI fetches them into a cache and runs this in its own job.
  if [[ -n "${AXIOVAL_PARITY_MODELS:-}" ]]; then
    cargo test -p axioval-cli --test parity -- --ignored --nocapture
  fi
  # Parity of expression rewrites on private models, likewise opt-in.
  if [[ -n "${AXIOVAL_PARITY_CASES:-}" ]]; then
    cargo test -p axioval --features ifc --test ifc_parity -- --ignored --nocapture
  fi
}

check_docs() {
  require mdbook
  RUSTDOCFLAGS="${RUSTDOCFLAGS:--D warnings}" cargo doc --workspace --all-features --no-deps
  mdbook build docs
}

sections=("$@")
if [[ ${#sections[@]} -eq 0 ]]; then
  sections=(deny lint test docs)
fi
for section in "${sections[@]}"; do
  case "$section" in
    deny | lint | test | docs) ;;
    *) printf 'unknown check section: %s (deny, lint, test, docs)\n' "$section" >&2; exit 2 ;;
  esac
done
for section in "${sections[@]}"; do
  printf '== check: %s\n' "$section"
  "check_$section"
done

printf 'all checks passed: %s\n' "${sections[*]}"
