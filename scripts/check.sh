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
#   test   workspace tests
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
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
}

check_test() {
  cargo test --workspace --all-features
}

check_docs() {
  require mdbook
  cargo doc --workspace --all-features --no-deps
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
