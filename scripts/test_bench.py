#!/usr/bin/env python3
"""Self-test of `bench.py`'s judgement: every way a template can break its
budget fails the gate, and a template within it passes. Runs no benchmark."""

from __future__ import annotations

import unittest

import bench

BUDGET = {"time": 1.25, "memory": 1.5, "floor_ns": 100_000}


def record(input: str = "fixture-400-walls.ifc", **changes: object) -> dict:
    entry = {
        "capability": "axioval:capability.body-extent",
        "input": input,
        "objects": 10,
        "parity": True,
        "differences": [],
        "template": {"median_ns": 1_100_000, "peak_bytes": 1_200},
        "reference": {"median_ns": 1_000_000, "peak_bytes": 1_000},
        "time_ratio": 1.1,
        "memory_ratio": 1.2,
    }
    entry.update(changes)
    return entry


def public(**changes: object) -> dict:
    return record("bs-sample.ifc", **changes)


class JudgeTest(unittest.TestCase):
    def failures(self, records: list[dict], gate: bool = True) -> list[str]:
        return bench.judge(records, BUDGET, gate)

    def test_the_tracked_budget_loads(self) -> None:
        budget = bench.load_budget()
        self.assertGreaterEqual(budget["time"], 1.0)
        self.assertGreaterEqual(budget["memory"], 1.0)

    def test_a_template_within_budget_passes(self) -> None:
        self.assertEqual(self.failures([record(), public()]), [])

    def test_a_slow_template_fails(self) -> None:
        failures = self.failures([record(time_ratio=1.3), public()])
        self.assertEqual(len(failures), 1)
        self.assertIn("run time", failures[0])

    def test_a_heavy_template_fails_even_on_a_small_input(self) -> None:
        small = {"median_ns": 10_000, "peak_bytes": 1_000}
        failures = self.failures(
            [record(), public(template=small, reference=small, time_ratio=1.0, memory_ratio=1.6)]
        )
        self.assertEqual(len(failures), 1)
        self.assertIn("peak heap", failures[0])

    def test_small_inputs_are_judged_by_their_sum(self) -> None:
        def small(template: int, reference: int) -> dict:
            return public(
                template={"median_ns": template, "peak_bytes": 1},
                reference={"median_ns": reference, "peak_bytes": 1},
                time_ratio=template / reference,
                memory_ratio=1.0,
            )

        # One noisy small input alone does not fail ...
        self.assertEqual(self.failures([record(), small(30_000, 20_000), small(20_000, 40_000)]), [])
        # ... but small inputs slow together do.
        failures = self.failures([record(), small(30_000, 20_000), small(30_000, 20_000)])
        self.assertEqual(len(failures), 1)
        self.assertIn("under the floor", failures[0])

    def test_a_parity_difference_fails(self) -> None:
        failures = self.failures([record(parity=False), public()])
        self.assertIn("parity", failures[0])

    def test_a_missing_public_model_fails_only_a_gate(self) -> None:
        missing = {"capability": "axioval:capability.body-extent", "input": "bs-x.ifc", "missing": True}
        self.assertEqual(len(self.failures([record(), public(), missing])), 1)
        self.assertEqual(self.failures([record(), public(), missing], gate=False), [])

    def test_a_gate_needs_a_fixture_and_a_public_model(self) -> None:
        self.assertIn("no public model was measured", self.failures([record()]))
        self.assertIn("no fixture was measured", self.failures([public()]))
        self.assertEqual(self.failures([], gate=False), ["the benchmark measured nothing"])

    def test_only_run_times_are_measured_again(self) -> None:
        self.assertTrue(bench.retimed(self.failures([record(time_ratio=1.3), public()])))
        heavy = self.failures([record(time_ratio=1.3, memory_ratio=1.6), public()])
        self.assertFalse(bench.retimed(heavy))
        self.assertFalse(bench.retimed([]))

    def test_the_table_lists_every_record(self) -> None:
        rendered = bench.table([record(), public(parity=False)])
        self.assertIn("| body-extent | fixture-400-walls.ifc |", rendered)
        self.assertIn("differs", rendered)


if __name__ == "__main__":
    unittest.main()
