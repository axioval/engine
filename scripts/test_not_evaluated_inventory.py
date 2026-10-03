#!/usr/bin/env python3
from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import not_evaluated_inventory as inventory


def obj(local_id: str, document: str = "m.ifc") -> dict:
    return {"source": {"system": "ifc-step", "document": document}, "local_id": local_id}


def result(unmeasured: list[tuple[str, str, str]], outcomes: list[dict]) -> dict:
    """A saved check result: unmeasured (id, kind, reason) and outcomes."""
    return {
        "report": {"findings": [], "not_evaluated": outcomes},
        "objects": {f"ifc-step:m.ifc/{i}": {"kind": kind} for i, kind, _ in unmeasured}
        | {"ifc-step:m.ifc/#9": {"kind": "IFCSPACE"}},
        "geometry": {
            "exact": 1,
            "tessellated": 0,
            "no_body": 0,
            "unmeasured": [{"object": obj(i), "reason": reason} for i, _, reason in unmeasured],
        },
    }


def outcome(rule: str, subject: str | None, reason: str, message: str) -> dict:
    return {
        "rule_id": rule,
        "object_id": obj(subject) if subject else None,
        "reason": reason,
        "message": message,
    }


RULES = {"clash": "clash", "pkg/area": "plan-area", "area": "plan-area"}


class PatternTests(unittest.TestCase):
    def test_references_ids_and_numbers_are_abstracted(self) -> None:
        self.assertEqual(
            inventory.pattern("pair with ifc-step:a.ifczip/b.ifc/#425 is 0.25 m apart, #7 too"),
            "pair with <object> is <n> m apart, #<id> too",
        )

    def test_names_with_digits_are_kept(self) -> None:
        self.assertEqual(inventory.pattern("IfcSweptDiskSolid in IFC2X3 at 1e-05"),
                         "IfcSweptDiskSolid in IFC2X3 at <n>")


class InventoryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.first = result(
            [("#1", "IFCWALL", "no body representation"),
             ("#2", "IFCBEAM", "refused: 3 paths")],
            [
                # The subject is unmeasured: blamed on its reason.
                outcome("clash", "#1", "incomplete_evidence", "proximity unavailable"),
                # A named counterpart is unmeasured: blamed on it.
                outcome("clash", "#5", "incomplete_evidence",
                        "proximity to ifc-step:m.ifc/#2 could not be measured"),
                # Nothing unmeasured involved: its own cause.
                outcome("pkg/area", "#9", "backend_unavailable", "footprint of #9: 4 vertices"),
            ],
        )
        self.second = result(
            [("#1", "IFCWALL", "no body representation")],
            [outcome("area", "#9", "backend_unavailable", "footprint of #12: 7 vertices")],
        )

    def test_outcomes_are_attributed_to_the_unmeasured_object_they_involve(self) -> None:
        causes = inventory.inventory([("a", self.first), ("b", self.second)], RULES)
        rows = {cause.label(): cause for cause in causes}
        self.assertEqual(rows["unmeasured: no body representation"].outcomes, 1)
        self.assertEqual(rows["unmeasured: no body representation"].objects, 2)
        self.assertEqual(rows["unmeasured: no body representation"].models, {"a", "b"})
        self.assertEqual(rows["unmeasured: refused: <n> paths"].outcomes, 1)
        own = rows["backend_unavailable · plan-area · footprint of #<id>: <n> vertices"]
        self.assertEqual(own.outcomes, 2)
        self.assertEqual(own.models, {"a", "b"})
        self.assertEqual(own.entities["IFCSPACE"], 2)
        self.assertEqual(sum(c.outcomes for c in causes), 4)

    def test_ranking_is_by_outcomes_then_objects_and_deterministic(self) -> None:
        causes = inventory.inventory([("a", self.first), ("b", self.second)], RULES)
        labels = [cause.label() for cause in causes]
        self.assertEqual(labels[0], "backend_unavailable · plan-area · footprint of #<id>: <n> vertices")
        self.assertEqual(labels[1], "unmeasured: no body representation")
        again = inventory.inventory([("b", self.second), ("a", self.first)], RULES)
        self.assertEqual(inventory.as_json(causes, ["a", "b"]),
                         inventory.as_json(again, ["a", "b"]))

    def test_rule_ids_map_to_capabilities_through_the_packages(self) -> None:
        rules = inventory.capabilities([inventory.DEFAULT_RULESET],
                                       [inventory.DEFAULT_DEFINITIONS])
        self.assertEqual(rules["element-clash"], "clash")
        self.assertEqual(rules["axioval:inventory.ruleset/element-clash"], "clash")

    def test_the_command_reads_labelled_results_and_rejects_duplicates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "x.json"
            path.write_text(json.dumps(self.first), encoding="utf-8")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(inventory.main(["--format", "json", f"m1={path}"]), 0)
            self.assertEqual(json.loads(out.getvalue())["models"], ["m1"])
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(inventory.main([str(path), str(path)]), 2)
                self.assertEqual(inventory.main([str(Path(directory) / "missing.json")]), 1)


if __name__ == "__main__":
    unittest.main()
