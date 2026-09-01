#!/usr/bin/env python3
"""Verify that every workspace crate archive carries required legal metadata."""
from __future__ import annotations

import json
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    "axioval",
    "axioval-axiolid",
    "axioval-cli",
    "axioval-engine",
    "axioval-icdd",
    "axioval-ir",
    "axioval-openbim",
    "axioval-rules",
}


def workspace_versions() -> dict[str, str]:
    output = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
        cwd=ROOT,
        text=True,
    )
    packages = {item["name"]: item["version"] for item in json.loads(output)["packages"]}
    missing = EXPECTED - packages.keys()
    if missing:
        raise ValueError(f"missing workspace packages: {', '.join(sorted(missing))}")
    return {name: packages[name] for name in EXPECTED}


def verify(package_dir: Path, versions: dict[str, str]) -> list[str]:
    errors: list[str] = []
    for package in sorted(EXPECTED):
        version = versions[package]
        archive = package_dir / f"{package}-{version}.crate"
        if not archive.is_file():
            errors.append(f"missing archive: {archive.name}")
            continue
        with tarfile.open(archive, "r:gz") as crate:
            root = f"{package}-{version}/"
            names = set(crate.getnames())
            for required in ("LICENSE", "README.md"):
                if root + required not in names:
                    errors.append(f"{archive.name}: missing {required}")
            manifest = crate.extractfile(root + "Cargo.toml")
            text = manifest.read().decode() if manifest is not None else ""
            if 'license = "AGPL-3.0-or-later"' not in text:
                errors.append(f"{archive.name}: missing SPDX license expression")
            if 'readme = "README.md"' not in text:
                errors.append(f"{archive.name}: missing README metadata")
    return errors


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        print(f"usage: {Path(argv[0]).name} [PACKAGE_DIR]", file=sys.stderr)
        return 2
    package_dir = Path(argv[1]).resolve() if len(argv) == 2 else ROOT / "target" / "package"
    try:
        versions = workspace_versions()
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"workspace metadata unavailable: {error}", file=sys.stderr)
        return 1
    errors = verify(package_dir, versions)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("package artifacts: LICENSE, README, and SPDX metadata ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
