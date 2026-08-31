#!/usr/bin/env bash
set -euo pipefail

export RUSTFLAGS=""

for tool in mdbook; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'required tool not found: %s\n' "$tool" >&2
    exit 1
  fi
done

if [[ "${AXIOVAL_DEPENDENCY_AUDIT_COMPLETE:-0}" != "1" ]]; then
  if ! command -v cargo-deny >/dev/null 2>&1; then
    printf 'required tool not found: cargo-deny\n' >&2
    exit 1
  fi
  cargo deny check
fi

python3 scripts/architecture.py --self-test
python3 scripts/migration.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
mdbook build docs

printf 'all checks passed\n'
