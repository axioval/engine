#!/usr/bin/env python3
"""Verify that every workspace crate archive carries required legal metadata."""
from __future__ import annotations

import json
import os
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
#
# The term list names those private parties, so hardcoding it here would be the
# very disclosure this check exists to prevent. It is loaded from a private,
# untracked file instead. Absence is FAIL, never skip: a publish gate that
# quietly disarms itself when its denylist goes missing is worse than no gate,
# because the one moment it disappears is the moment a leak ships.
DEFAULT_TERMS_FILE = ROOT / "private" / "forbidden-terms.json"

# Opting out of the term scan is explicit and per-invocation. A public
# checkout -- CI included -- cannot hold the denylist, but inferring the
# relaxation from its absence would silently disarm the publish gate on the
# maintainer's machine too, the one place it must bite. Forgetting the flag
# fails closed; passing it is a visible choice in the caller.
STRUCTURE_ONLY_FLAG = "--structure-only"


def forbidden_terms() -> tuple[str, ...]:
    configured = os.environ.get("AXIOVAL_FORBIDDEN_TERMS", "").strip()
    path = Path(configured) if configured else DEFAULT_TERMS_FILE
    if not path.is_file():
        raise ValueError(
            f"forbidden-term list not found at {path}; set AXIOVAL_FORBIDDEN_TERMS. "
            "Refusing to verify a package with an empty denylist."
        )
    terms = json.loads(path.read_text(encoding="utf-8")).get("terms")
    if not isinstance(terms, list) or not terms or not all(isinstance(t, str) and t for t in terms):
        raise ValueError(f"{path}: `terms` must be a non-empty list of strings")
    return tuple(term.casefold() for term in terms)


# A crate's LICENSE is a relative symlink to the workspace root. Moving a crate
# between directories changes its depth and silently dangles that link, which
# only surfaces once archives are built -- by which point a crate can already
# have been published without its license. Checked directly on the source tree
# so it fails in seconds, before packaging.
def dangling_license_links() -> list[str]:
    errors: list[str] = []
    # `attic/` is scanned too: retired crates are still publishable
    # artifacts carrying the same relative-symlink LICENSE. Being outside
    # the workspace is what makes them easy to forget, not what makes them
    # safe -- a retired crate can be republished with a dangling license.
    roots = (ROOT / "crates", ROOT / "attic")
    manifests = sorted(m for root in roots if root.is_dir() for m in root.rglob("Cargo.toml"))
    for manifest in manifests:
        crate = manifest.parent
        if "[package]" not in manifest.read_text(encoding="utf-8"):
            continue
        link = crate / "LICENSE"
        if not link.is_file():
            target = link.readlink() if link.is_symlink() else "missing"
            errors.append(f"{link.relative_to(ROOT)}: unreadable LICENSE ({target})")
    return errors


def verify(package_dir: Path, versions: dict[str, str], terms: tuple[str, ...]) -> list[str]:
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
                for term in terms:
                    if term in body.casefold():
                        errors.append(
                            f"{archive.name}: {member.name} names a forbidden "
                            f"downstream party"
                        )
            manifest = crate.extractfile(root + "Cargo.toml")
            text = manifest.read().decode() if manifest is not None else ""
            if 'license = "AGPL-3.0-or-later"' not in text:
                errors.append(f"{archive.name}: missing SPDX license expression")
            if 'readme = "README.md"' not in text:
                errors.append(f"{archive.name}: missing README metadata")
    return errors


def main(argv: list[str]) -> int:
    args = [a for a in argv[1:] if a != STRUCTURE_ONLY_FLAG]
    structure_only = STRUCTURE_ONLY_FLAG in argv[1:]
    if len(args) > 1:
        print(f"usage: {Path(argv[0]).name} [{STRUCTURE_ONLY_FLAG}] [PACKAGE_DIR]", file=sys.stderr)
        return 2
    package_dir = Path(args[0]).resolve() if args else ROOT / "target" / "package"
    try:
        versions = workspace_versions()
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"workspace metadata unavailable: {error}", file=sys.stderr)
        return 1
    terms: tuple[str, ...] = ()
    if structure_only:
        print("structure-only: skipping the forbidden-term scan", file=sys.stderr)
    else:
        try:
            terms = forbidden_terms()
        except ValueError as error:
            print(error, file=sys.stderr)
            return 1
    errors = dangling_license_links()
    errors += verify(package_dir, versions, terms)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("package artifacts: LICENSE, README, and SPDX metadata ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
