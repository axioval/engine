#!/usr/bin/env python3
"""Fail if staging work has leaked into the shipping workspace.

Staging crates depend on unpublished Axiolid crates by path. That is fine in
isolation and fatal in the workspace: a path dependency cannot be published, so
a staging crate becoming a member would break every release rather than just
itself. This check makes that leak loud instead of discovering it at publish
time.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STAGING = ROOT / "staging"
EXPECTED_MEMBERS = 9


def failures() -> list[str]:
    problems: list[str] = []
    if not STAGING.exists():
        return problems

    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    members = {p["name"]: p["manifest_path"] for p in metadata["packages"]}

    if len(members) != EXPECTED_MEMBERS:
        problems.append(
            f"workspace has {len(members)} crates, expected {EXPECTED_MEMBERS}"
        )

    # No workspace member may live under staging/.
    for name, manifest in members.items():
        if str(STAGING) in manifest:
            problems.append(f"staging crate '{name}' is a workspace member")

    # Every staging crate must be explicitly unpublishable.
    for manifest in STAGING.glob("*/Cargo.toml"):
        text = manifest.read_text(encoding="utf-8")
        if "publish = false" not in text:
            problems.append(f"{manifest.relative_to(ROOT)} lacks 'publish = false'")
        if "[workspace]" not in text:
            problems.append(
                f"{manifest.relative_to(ROOT)} lacks an empty [workspace] table, "
                "so it would be absorbed into the engine workspace"
            )

    # A published crate must never depend on a staging crate.
    for manifest in ROOT.glob("crates/**/Cargo.toml"):
        if "staging/" in manifest.read_text(encoding="utf-8"):
            problems.append(
                f"{manifest.relative_to(ROOT)} depends on a staging crate"
            )

    return problems


def main() -> int:
    problems = failures()
    for problem in problems:
        print(f"staging isolation: {problem}", file=sys.stderr)
    if problems:
        return 1
    print("staging isolation: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
