#!/usr/bin/env python3
"""Fetch the public IFC sample models the parity harness runs on.

The models are openly licensed but not vendored: `fixtures/parity/models.json`
lists each by a URL pinned to a commit, its SHA-256 and its licence. This
script downloads them into a cache directory and verifies every digest, so a
model is used only as pinned.

    scripts/parity_models.py fetch [DIR]   download and verify; print DIR
    scripts/parity_models.py check         validate the manifest, offline

DIR defaults to `AXIOVAL_PARITY_MODELS`, else `$XDG_CACHE_HOME/axioval/
parity-models`, else `~/.cache/axioval/parity-models`: on disk, never in a
RAM-backed temporary directory. A file already there with the pinned digest
is kept; one with another digest is replaced. A download whose digest differs
from the pin fails the fetch and is not kept.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "fixtures" / "parity" / "models.json"
# Licences a public model may carry: open, and allowing redistribution of
# test results about the model.
LICENCES = {"CC-BY-4.0"}
PINNED = re.compile(r"^https://raw\.githubusercontent\.com/[\w.-]+/[\w.-]+/[0-9a-f]{40}/\S+$")
NAME = re.compile(r"^[a-z0-9][a-z0-9.-]*\.ifc$")
DIGEST = re.compile(r"^[0-9a-f]{64}$")


def load() -> list[dict]:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    models = data.get("models")
    if not isinstance(models, list) or not models:
        raise SystemExit(f"{MANIFEST}: `models` must list at least one model")
    return models


def check(models: list[dict]) -> None:
    names = set()
    for model in models:
        name = model.get("name", "")
        if not NAME.match(name):
            raise SystemExit(f"{MANIFEST}: model name {name!r} is not a lowercase `.ifc` file name")
        if name in names:
            raise SystemExit(f"{MANIFEST}: model {name} listed twice")
        names.add(name)
        if not PINNED.match(model.get("url", "")):
            raise SystemExit(f"{MANIFEST}: {name}: the URL must be pinned to a commit")
        if not DIGEST.match(model.get("sha256", "")):
            raise SystemExit(f"{MANIFEST}: {name}: `sha256` must be 64 lowercase hex digits")
        if model.get("licence") not in LICENCES:
            raise SystemExit(f"{MANIFEST}: {name}: licence {model.get('licence')!r} is not one of {sorted(LICENCES)}")
        if not model.get("attribution"):
            raise SystemExit(f"{MANIFEST}: {name}: an attribution is required")


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cache_dir(argument: str | None) -> Path:
    if argument:
        return Path(argument)
    configured = os.environ.get("AXIOVAL_PARITY_MODELS", "").strip()
    if configured:
        return Path(configured)
    base = os.environ.get("XDG_CACHE_HOME", "").strip() or str(Path.home() / ".cache")
    return Path(base) / "axioval" / "parity-models"


def fetch(models: list[dict], directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    for model in models:
        target = directory / model["name"]
        if target.is_file() and digest(target) == model["sha256"]:
            continue
        with urllib.request.urlopen(model["url"], timeout=120) as response:
            body = response.read()
        actual = hashlib.sha256(body).hexdigest()
        if actual != model["sha256"]:
            raise SystemExit(f"{model['name']}: downloaded SHA-256 {actual}, pinned {model['sha256']}")
        # Written beside the target, then renamed: never a half-written model.
        with tempfile.NamedTemporaryFile(dir=directory, delete=False) as part:
            part.write(body)
        Path(part.name).replace(target)
    print(directory)


def main() -> None:
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    models = load()
    check(models)
    if command == "check":
        print(f"parity models: {len(models)} pinned")
    elif command == "fetch":
        fetch(models, cache_dir(sys.argv[2] if len(sys.argv) > 2 else None))
    else:
        raise SystemExit("usage: parity_models.py fetch [DIR] | check")


if __name__ == "__main__":
    main()
