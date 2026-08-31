#!/usr/bin/env python3
"""Validate the Solibri migration ledger fail closed."""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "migration" / "solibri-capabilities.json"
ALLOWED = {"pending", "in_progress", "ported", "blocked"}
EXPECTED = 65


def main() -> None:
    data = json.loads(LEDGER.read_text(encoding="utf-8"))
    entries = data.get("entries")
    if not isinstance(entries, list) or len(entries) != EXPECTED:
        raise SystemExit(f"expected {EXPECTED} migration entries, found {len(entries) if isinstance(entries, list) else 'invalid'}")
    names: set[str] = set()
    for entry in entries:
        name = entry.get("nativeType")
        if not isinstance(name, str) or not name or name in names:
            raise SystemExit(f"invalid or duplicate nativeType: {name!r}")
        names.add(name)
        status = entry.get("status")
        if status not in ALLOWED:
            raise SystemExit(f"{name}: invalid status {status!r}")
        proof = entry.get("proof")
        if not isinstance(proof, list):
            raise SystemExit(f"{name}: proof must be a list")
        if status == "ported" and len(proof) < len(data["completionContract"]):
            raise SystemExit(f"{name}: ported without all proof obligations")
        if status == "blocked" and not entry.get("blocker"):
            raise SystemExit(f"{name}: blocked without an explicit blocker")
    print(f"migration ledger: {len(entries)} entries valid")


if __name__ == "__main__":
    main()
