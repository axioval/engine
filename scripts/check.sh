#!/usr/bin/env bash
set -euo pipefail

export RUSTFLAGS=""

python3 scripts/architecture.py --self-test
python3 scripts/migration.py
cargo deny check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
mdbook build docs

printf 'all checks passed\n'
