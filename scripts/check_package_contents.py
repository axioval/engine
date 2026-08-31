#!/usr/bin/env python3
"""Verify that every workspace crate archive carries required legal metadata."""
from __future__ import annotations

import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGE_DIR = ROOT / "target" / "package"
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

errors: list[str] = []
for package in sorted(EXPECTED):
    archive = PACKAGE_DIR / f"{package}-0.1.0.crate"
    if not archive.is_file():
        errors.append(f"missing archive: {archive.name}")
        continue
    with tarfile.open(archive, "r:gz") as crate:
        root = f"{package}-0.1.0/"
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

if errors:
    print("\n".join(errors), file=sys.stderr)
    raise SystemExit(1)
print("package artifacts: LICENSE, README, and SPDX metadata ok")
