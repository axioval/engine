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

    def test_no_shape_is_model_data_apart_from_no_body(self) -> None:
        shapes = result(
            [("#1", "IFCBUILDINGELEMENTPROXY", "no shape representation"),
             ("#2", "IFCBUILDINGELEMENTPROXY", "no shape representation"),
             ("#3", "IFCMEMBER", "no body representation; it has Axis"),
             ("#4", "IFCBEAM", "no body representation; it has Axis, FootPrint")],
            [outcome("clash", "#1", "incomplete_evidence", "proximity unavailable"),
             outcome("clash", "#3", "incomplete_evidence", "proximity unavailable")],
        )
        causes = inventory.inventory([("a", shapes)], RULES)
        rows = {cause.label(): cause for cause in causes}
        self.assertEqual(
            sorted(rows),
            ["unmeasured (model data): no shape representation",
             "unmeasured: no body representation; it has <identifiers>"],
        )
        shapeless = rows["unmeasured (model data): no shape representation"]
        self.assertEqual((shapeless.outcomes, shapeless.objects), (1, 2))
        bodiless = rows["unmeasured: no body representation; it has <identifiers>"]
        self.assertEqual((bodiless.outcomes, bodiless.objects), (1, 2))
        rendered = json.loads(inventory.as_json(causes, ["a"]))["causes"]
        self.assertEqual(
            {row["cause"]: row["model_data"] for row in rendered},
            {"unmeasured (model data): no shape representation": True,
             "unmeasured: no body representation; it has <identifiers>": False},
        )

    def test_a_face_crossing_itself_is_model_data_but_a_keyhole_is_not(self) -> None:
        refused = "mesh compilation refused: "
        faces = result(
            [("#1", "IFCPIPEFITTING", refused + "invalid geometry input: planar face cannot "
              "be triangulated: profile outer ring intersects itself"),
             ("#2", "IFCBUILDINGELEMENTPROXY", refused + "invalid geometry input: authored "
              "polygon face 0 cannot be triangulated: its rings do not bound a region: "
              "profile outer ring intersects itself"),
             ("#3", "IFCWALL", refused + "invalid geometry input: planar face cannot be "
              "triangulated: profile outer ring folds back on itself at vertex 6"),
             ("#4", "IFCSANITARYTERMINAL", refused + "invalid geometry input: authored "
              "polygon face 21 cannot be triangulated: its rings do not bound a region: "
              "profile outer ring overlaps itself"),
             ("#5", "IFCDOOR", refused + "numerically degenerate input: planar face cannot "
              "be triangulated: profile triangulation found no ear among 12 remaining "
              "vertices")],
            [],
        )
        causes = inventory.inventory([("a", faces)], RULES)
        labelled = sorted(cause.label() for cause in causes if cause.model_data())
        self.assertEqual(len(labelled), 3, labelled)
        self.assertTrue(all(label.startswith("unmeasured (model data): ") for label in labelled))
        others = sorted(cause.text for cause in causes if not cause.model_data())
        self.assertEqual(len(others), 2, others)
        self.assertTrue(others[0].endswith("overlaps itself"), others)
        self.assertTrue(others[1].endswith("found no ear among <n> remaining vertices"), others)

    def test_openings_removing_the_whole_body_are_model_data(self) -> None:
        voided = result(
            [("#1", "IFCWALLSTANDARDCASE", "its openings remove its whole body: nothing is "
              "left once the 1 opening(s) voiding it are subtracted"),
             ("#2", "IFCBUILDINGELEMENTPROXY", "its openings remove its whole body: nothing "
              "is left once the 2 opening(s) voiding it are subtracted"),
             ("#3", "IFCWALL", "mesh compilation produced no triangles")],
            [outcome("clash", "#1", "incomplete_evidence", "proximity unavailable")],
        )
        causes = inventory.inventory([("a", voided)], RULES)
        rows = {cause.label(): cause for cause in causes}
        self.assertEqual(
            sorted(rows),
            ["unmeasured (model data): its openings remove its whole body: nothing is left "
             "once the <n> opening(s) voiding it are subtracted",
             "unmeasured: mesh compilation produced no triangles"],
        )
        self.assertEqual(
            [(cause.outcomes, cause.objects) for cause in causes if cause.model_data()],
            [(1, 2)],
        )

    def test_invalid_geometry_is_model_data(self) -> None:
        invalid = result(
            [("#1", "IFCWALL", "#1906924 (IFCCOMPOSITECURVE) is geometrically invalid: "
              "segment 2 starts 0.83 m from where segment 1 ends"),
             ("#2", "IFCSLAB", "#12 (IFCPOLYLINE) is geometrically invalid: point #14 lies "
              "off the boundary plane (BoundaryDim)"),
             ("#3", "IFCWALL", "IFCTRIMMEDCURVE (#5) is valid IFC but not yet interpreted: "
              "an arc")],
            [outcome("clash", "#1", "incomplete_evidence", "proximity unavailable")],
        )
        causes = inventory.inventory([("a", invalid)], RULES)
        rows = {cause.label(): (cause.outcomes, cause.objects) for cause in causes}
        self.assertEqual(
            rows,
            {"unmeasured (model data): #<id> (IFCCOMPOSITECURVE) is geometrically invalid: "
             "segment <n> starts <n> m from where segment <n> ends": (1, 1),
             "unmeasured (model data): #<id> (IFCPOLYLINE) is geometrically invalid: point "
             "#<id> lies off the boundary plane (BoundaryDim)": (0, 1),
             "unmeasured: IFCTRIMMEDCURVE (#<id>) is valid IFC but not yet interpreted: an "
             "arc": (0, 1)},
        )

    def test_a_whole_unmeasured_through_a_part_is_model_data_when_the_part_is(self) -> None:
        whole = ("no body representation of its own, and its body is the union of its {} "
                 "parts, and part ifc-step:m.ifc/#2 is unmeasured: {}")
        crossing = ("mesh compilation refused: invalid geometry input: planar face cannot be "
                    "triangulated: profile outer ring intersects itself")
        refused = "mesh compilation refused: backend cannot apply Sweep"
        wholes = result(
            [("#1", "IFCRAILING", whole.format(99, crossing)),
             ("#2", "IFCMEMBER", crossing),
             ("#3", "IFCSTAIR", whole.format(2, refused)),
             # A whole of wholes: through its part's part.
             ("#4", "IFCSTAIR", whole.format(1, whole.format(99, crossing)).replace(
                 "parts, and", "part, and", 1))],
            [outcome("clash", "#1", "incomplete_evidence", "proximity unavailable"),
             outcome("clash", "#3", "incomplete_evidence", "proximity unavailable")],
        )
        causes = inventory.inventory([("a", wholes)], RULES)
        rows = {cause.label(): (cause.outcomes, cause.objects) for cause in causes}
        through = ("no body representation of its own, and its body is the union of its <n> "
                   "parts, and part <object> is unmeasured: ")
        self.assertEqual(
            rows,
            {"unmeasured (model data): " + through + inventory.pattern(crossing): (1, 1),
             "unmeasured (model data): " + inventory.pattern(crossing): (0, 1),
             "unmeasured: " + through + inventory.pattern(refused): (1, 1),
             "unmeasured (model data): " + through.replace("parts,", "part,")
             + through + inventory.pattern(crossing): (0, 1)},
        )

    def test_bodies_authored_open_are_model_data_only_when_all_are(self) -> None:
        meet = ("surfaces meet ifc-step:m.ifc/{}, but neither body is a closed solid, so "
                "touching cannot be told from crossing")
        below = ("free-space query unavailable: ifc-step:m.ifc/{} reaches below the headroom "
                 "band but is not a closed solid, so what it occupies in the band is undecided")
        faces = result(
            [],
            [outcome("clash", "#1", "incomplete_evidence", meet.format("#2")),
             outcome("clash", "#1", "incomplete_evidence", meet.format("#3")),
             outcome("area", "#9", "backend_unavailable", below.format("#2")),
             outcome("area", "#9", "backend_unavailable", below.format("#3"))],
        )
        faces["integrity"] = [
            {"code": "shape.open-surface", "severity": "warning", "message": "",
             "locator": f"ifc:sha256:0:open-surface:{local}"}
            for local in ("#1", "#2")
        ] + [{"code": "shape.no-representation", "locator": "ifc:sha256:0:no-shape:#3"}]
        causes = inventory.inventory([("a", faces)], RULES)
        rows = {cause.label(): (cause.outcomes, cause.model_data()) for cause in causes}
        clash = "incomplete_evidence{} · clash · " + inventory.pattern(meet.format("#2"))
        free = "backend_unavailable{} · plan-area · " + inventory.pattern(below.format("#2"))
        self.assertEqual(
            rows,
            {clash.format(" (model data)"): (1, True), clash.format(""): (1, False),
             free.format(" (model data)"): (1, True), free.format(""): (1, False)},
        )


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
