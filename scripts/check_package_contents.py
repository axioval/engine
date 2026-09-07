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
    "axioval-ifc",
    "axioval-ir",
    "axioval-rules",
    "axioval-spec",
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


# Downstream consumers and vendor products must never appear in a published
# crate. Axioval is source-neutral: naming a particular vendor in shipped text
# both leaks a private integration and contradicts the neutrality the crates
# claim. Compared case-insensitively against every shipped text file.
FORBIDDEN_TERMS = ("the provider",)


# A crate's LICENSE is a relative symlink to the workspace root. Moving a crate
# between directories changes its depth and silently dangles that link, which
# only surfaces once archives are built -- by which point a crate can already
# have been published without its license. Checked directly on the source tree
# so it fails in seconds, before packaging.
def dangling_license_links() -> list[str]:
    errors: list[str] = []
    for manifest in sorted((ROOT / "crates").rglob("Cargo.toml")):
        crate = manifest.parent
        if "[package]" not in manifest.read_text(encoding="utf-8"):
            continue
        link = crate / "LICENSE"
        if not link.is_file():
            target = link.readlink() if link.is_symlink() else "missing"
            errors.append(f"{link.relative_to(ROOT)}: unreadable LICENSE ({target})")
    return errors


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
            # A published artifact must not name a downstream vendor. Read
            # every shipped text member, not just the manifest: the leak that
            # motivated this was in READMEs and doc comments, which are the
            # most visible files on a registry page.
            for member in crate.getmembers():
                if not member.isfile():
                    continue
                handle = crate.extractfile(member)
                if handle is None:
                    continue
                try:
                    body = handle.read().decode("utf-8")
                except UnicodeDecodeError:
                    continue
                for term in FORBIDDEN_TERMS:
                    if term in body.casefold():
                        errors.append(
                            f"{archive.name}: {member.name} names forbidden "
                            f"downstream vendor {term!r}"
                        )
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
    errors = dangling_license_links()
    errors += verify(package_dir, versions)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("package artifacts: LICENSE, README, and SPDX metadata ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
