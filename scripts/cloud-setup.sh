#!/usr/bin/env bash
# Cloud environment setup script for Axioval engine sessions.
#
# Paste this file's contents (or `bash scripts/cloud-setup.sh` after cloning)
# into the environment's setup script. It is idempotent.
#
# 1. Installs the pinned Rust toolchain and the gate's extra tools.
# 2. Writes user-level Claude settings so no commit, issue or pull request
#    ever carries a generated-with footer or a claude.ai session link.
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

rustup toolchain install 1.88.0 --profile minimal --component clippy --component rustfmt
command -v cargo-deny >/dev/null 2>&1 || cargo install --locked cargo-deny
command -v mdbook >/dev/null 2>&1 || cargo install --locked mdbook

mkdir -p "$HOME/.claude"
settings="$HOME/.claude/settings.json"
python3 - "$settings" <<'PY'
import json, os, sys

path = sys.argv[1]
data = {}
if os.path.exists(path):
    with open(path) as f:
        data = json.load(f)
data["attribution"] = {"commit": "", "pr": ""}
data["includeCoAuthoredBy"] = False
with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PY
