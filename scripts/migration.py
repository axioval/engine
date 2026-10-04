#!/usr/bin/env python3
"""Validate the private capability-migration ledger fail closed.

The ledger names a private downstream consumer, so it is not tracked in this
repository. It lives under `private/` (gitignored) or wherever
`AXIOVAL_MIGRATION_LEDGER` points. When it is absent -- a public clone, CI --
this check reports that it was skipped and exits clean: an absent private file
is not a repository defect. When it is present it is validated strictly, so a
maintainer can never promote a capability without its proof obligations.
"""

from __future__ import annotations

import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ALLOWED = {"pending", "in_progress", "ported", "blocked"}
EXPECTED = 65


def ledger_path() -> Path:
    configured = os.environ.get("AXIOVAL_MIGRATION_LEDGER", "").strip()
    return Path(configured) if configured else ROOT / "private" / "capabilities.json"


def validate_parity(name: str, item: object) -> None:
    """A parity proof is the evidence the parity harness prints: it names
    both rules and records no difference. Any other proof item passes."""
    if not isinstance(item, dict) or item.get("kind") != "parity":
        return
    for field in ("capability", "expression"):
        if not isinstance(item.get(field), str) or not item[field]:
            raise SystemExit(f"{name}: parity proof without a {field} rule")
    if not isinstance(item.get("objects"), int) or item["objects"] < 1:
        raise SystemExit(f"{name}: parity proof over no objects")
    if item.get("differences") != []:
        raise SystemExit(f"{name}: parity proof records differences")


def validate(data: dict) -> None:
    entries = data.get("entries")
    if not isinstance(entries, list) or len(entries) != EXPECTED:
        raise SystemExit(
            f"expected {EXPECTED} migration entries, "
            f"found {len(entries) if isinstance(entries, list) else 'invalid'}"
        )
    contract = data.get("completionContract")
    if not isinstance(contract, list) or not contract:
        raise SystemExit("completionContract must be a non-empty list")
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
        for item in proof:
            validate_parity(name, item)
        if status == "ported" and len(proof) < len(contract):
            raise SystemExit(f"{name}: ported without all proof obligations")
        if status == "blocked" and not entry.get("blocker"):
            raise SystemExit(f"{name}: blocked without an explicit blocker")
    print(f"migration ledger: {len(entries)} entries valid")


def main() -> None:
    path = ledger_path()
    if not path.is_file():
        print(f"migration ledger: not present at {path}; skipped")
        return
    validate(json.loads(path.read_text(encoding="utf-8")))


if __name__ == "__main__":
    main()
