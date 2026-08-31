#!/usr/bin/env python3
"""Print a reproducible SHA-256 identity for the reviewable repository snapshot."""
from __future__ import annotations

import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXCLUDED_PARTS = {".git", "target"}
EXCLUDED_PATHS = {Path("docs/book")}


def included(path: Path) -> bool:
    relative = path.relative_to(ROOT)
    return not any(part in EXCLUDED_PARTS for part in relative.parts) and not any(
        relative == excluded or excluded in relative.parents for excluded in EXCLUDED_PATHS
    )


def main() -> None:
    digest = hashlib.sha256()
    for path in sorted(path for path in ROOT.rglob("*") if path.is_file() and included(path)):
        relative = path.relative_to(ROOT).as_posix().encode()
        content = path.read_bytes()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
    print(digest.hexdigest())


if __name__ == "__main__":
    main()
