#!/usr/bin/env python3
"""Self-test of `bench.py`'s judgement: every way a template can break its
budget fails the gate, a template within it passes, and a run time too close
to call is measured in further rounds and never passes undecided. Runs no
benchmark."""

from __future__ import annotations

import json
import math
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import bench

BUDGET = {
    "time": 1.25,
    "memory": 1.5,
    "floor_ns": 100_000,
    "small_slack_ns": 50_000,
    "slack_bytes": 65_536,
    "confidence": 0.99,
    "looks": [2, 3, 6],
}


def sides(ratios: list[float], reference_ns: int = 1_000_000) -> tuple[dict, dict]:
    """A template and a reference side whose paired runs have `ratios`."""
    template = [round(reference_ns * ratio) for ratio in ratios]
    reference = [reference_ns] * len(ratios)

    def side(times: list[int], peak: int) -> dict:
        return {
            "runs": len(times),
            "median_ns": sorted(times)[len(times) // 2],
            "peak_bytes": peak,
            "times_ns": times,
            "peaks_bytes": [peak] * len(times),
        }

    return side(template, 1_200), side(reference, 1_000)


def noisy(median: float, runs: int = 21) -> list[float]:
    """One round of `runs` paired ratios around `median`, a few far off."""
    ratios = [median * (1 + 0.002 * (index - runs // 2)) for index in range(runs)]
    ratios[3] = 3 * median  # one run slowed by the machine ...
    ratios[17] = median / 3  # ... and one sped up, so the median stays
    return ratios


def record(input: str = "fixture-400-walls.ifc", medians: list[float] | None = None, **changes: object) -> dict:
    """A record measured in one round per entry of `medians`."""
    medians = medians if medians is not None else [1.10, 1.11, 1.09]
    ratios = [ratio for median in medians for ratio in noisy(median)]
    template, reference = sides(ratios)
    entry = {
        "capability": "axioval:capability.body-extent",
        "input": input,
        "objects": 10,
        "parity": True,
        "differences": [],
        "template": template,
        "reference": reference,
        "time_ratio": template["median_ns"] / reference["median_ns"],
        "memory_ratio": 1.2,
        "rounds": [21] * len(medians),
    }
    entry.update(changes)
    return entry


def public(**changes: object) -> dict:
    return record("bs-sample.ifc", **changes)


def verdict(entry: dict, budget: dict = BUDGET) -> str:
    return bench.time_judgement(entry, budget)["verdict"]


class IntervalTest(unittest.TestCase):
    def test_student_quantiles(self) -> None:
        for probability, df, expected in ((0.995, 1, 63.657), (0.995, 2, 9.925), (0.995, 9, 3.250), (0.975, 4, 2.776)):
            self.assertAlmostEqual(bench.student_quantile(probability, df), expected, places=3)

    def test_each_round_counts_once(self) -> None:
        entry = record(medians=[1.1, 1.2, 1.3])
        self.assertEqual([round(math.exp(m), 6) for m in bench.round_medians(entry)], [1.1, 1.2, 1.3])
        self.assertEqual(bench.time_judgement(entry, BUDGET)["rounds"], 3)
        # The interval is the t interval over the rounds' log medians.
        logs = [math.log(m) for m in (1.1, 1.2, 1.3)]
        centre = sum(logs) / 3
        width = 9.925 * math.sqrt(sum((x - centre) ** 2 for x in logs) / 2) / math.sqrt(3)
        judged = bench.time_judgement(entry, BUDGET)
        self.assertAlmostEqual(judged["upper"], math.exp(centre + width), places=3)
        self.assertAlmostEqual(judged["lower"], math.exp(centre - width), places=3)

    def test_rounds_that_do_not_count_the_runs_are_refused(self) -> None:
        with self.assertRaises(SystemExit):
            bench.round_medians(record(rounds=[21, 20]))


class JudgeTest(unittest.TestCase):
    def failures(self, records: list[dict], gate: bool = True) -> list[str]:
        return bench.judge(records, BUDGET, gate)

    def test_the_tracked_budget_loads(self) -> None:
        budget = bench.load_budget()
        self.assertGreaterEqual(budget["time"], 1.0)
        self.assertGreaterEqual(budget["memory"], 1.0)
        self.assertLess(budget["confidence"], 1.0)
        self.assertGreaterEqual(budget["looks"][0], 2)
        self.assertEqual(budget["looks"], sorted(set(budget["looks"])))

    def test_a_template_within_budget_passes(self) -> None:
        self.assertEqual(verdict(record()), bench.PASS)
        self.assertEqual(self.failures([record(), public()]), [])

    def test_a_slow_template_fails(self) -> None:
        slow = record(medians=[1.40, 1.41, 1.39])
        self.assertEqual(verdict(slow), bench.FAIL)
        failures = self.failures([slow, public()])
        self.assertEqual(len(failures), 1)
        self.assertIn("exceeds", failures[0])

    def test_a_template_near_its_ceiling_is_undecided(self) -> None:
        # Over the ceiling by its median, under it by its interval ...
        over = record(medians=[1.24, 1.28, 1.27])
        self.assertEqual(verdict(over), bench.UNDECIDED)
        failures = self.failures([over, public()])
        self.assertEqual(len(failures), 1)
        self.assertIn("undecided", failures[0])
        # ... and under it by its median, over it by its interval: neither passes.
        self.assertEqual(verdict(record(medians=[1.21, 1.25, 1.23])), bench.UNDECIDED)

    def test_one_round_decides_nothing(self) -> None:
        # However far from the ceiling: one process's runs share its layout.
        self.assertEqual(verdict(record(medians=[0.5])), bench.UNDECIDED)
        self.assertEqual(verdict(record(medians=[2.0])), bench.UNDECIDED)
        old = record(medians=[1.1])
        del old["rounds"]
        for side in ("template", "reference"):
            del old[side]["times_ns"]
        self.assertEqual(verdict(old), bench.UNDECIDED)

    def test_a_slow_run_does_not_decide_a_round(self) -> None:
        self.assertAlmostEqual(math.exp(bench.round_medians(record(medians=[1.1]))[0]), 1.1)

    def test_only_undecided_inputs_with_rounds_left_are_measured_again(self) -> None:
        close = record(medians=[1.24, 1.28, 1.27])
        slow = public(medians=[1.4, 1.41, 1.39])
        first = public(medians=[0.5])
        self.assertEqual(bench.undecided([record(), close, slow, first], BUDGET), [close, first])
        exhausted = record(medians=[1.24, 1.28, 1.27, 1.22, 1.26, 1.25])
        self.assertEqual(bench.undecided([exhausted], BUDGET), [])
        # Still undecided at the bound: it fails.
        self.assertIn("undecided", self.failures([exhausted, public()])[0])

    def test_a_heavy_template_fails_even_on_a_small_input(self) -> None:
        failures = self.failures(
            [
                record(),
                public(
                    template={"median_ns": 10_000, "peak_bytes": 200_000},
                    reference={"median_ns": 10_000, "peak_bytes": 100_000},
                    time_ratio=1.0,
                    memory_ratio=2.0,
                ),
            ]
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
        # ... but small inputs slow together, beyond their slack, do.
        failures = self.failures([record(), small(99_000, 40_000), small(99_000, 40_000)])
        self.assertEqual(len(failures), 1)
        self.assertIn("under the floor", failures[0])

    def test_a_heap_within_the_slack_passes(self) -> None:
        # 2x of a few kibibytes is a fixed cost, not a regression.
        tiny = public(
            template={"median_ns": 10_000, "peak_bytes": 40_000},
            reference={"median_ns": 10_000, "peak_bytes": 20_000},
            time_ratio=1.0,
            memory_ratio=2.0,
        )
        self.assertEqual(self.failures([record(), tiny]), [])

    def test_small_inputs_within_the_slack_pass(self) -> None:
        def small(template: int, reference: int) -> dict:
            return public(
                template={"median_ns": template, "peak_bytes": 1},
                reference={"median_ns": reference, "peak_bytes": 1},
                time_ratio=template / reference,
                memory_ratio=1.0,
            )

        # 1.6x of 40 us is 24 us over: within 50 us per input ...
        self.assertEqual(self.failures([record(), small(64_000, 40_000)]), [])
        # ... while 110 us over two small inputs is not.
        failures = self.failures([record(), small(95_000, 40_000), small(95_000, 40_000)])
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

    def test_a_recorded_exception_has_its_own_ceiling(self) -> None:
        budget = dict(BUDGET, exceptions=[{
            "capability": "axioval:capability.body-extent",
            "input": "fixture-400-walls.ifc",
            "time": 1.5,
            "reason": "services that cost nothing",
        }])
        self.assertEqual(bench.judge([record(medians=[1.4, 1.41, 1.39]), public()], budget, True), [])
        self.assertEqual(len(bench.judge([record(medians=[1.6, 1.61, 1.59]), public()], budget, True)), 1)
        # Only on its own input.
        self.assertEqual(len(bench.judge([record(), public(medians=[1.4, 1.41, 1.39])], budget, True)), 1)

    def test_the_table_lists_every_record_with_its_spread(self) -> None:
        rendered = bench.table([record(), public(parity=False)], BUDGET)
        self.assertIn("| body-extent | fixture-400-walls.ifc |", rendered)
        self.assertIn("differs", rendered)
        self.assertIn("| 3 | pass |", rendered)
        self.assertIn("| 0.9% |", rendered)


class SequentialTest(unittest.TestCase):
    def test_merging_pools_a_round(self) -> None:
        first = record(medians=[1.24, 1.28])
        again = record(medians=[1.0], parity=False)
        del again["rounds"]  # as the bench writes it
        [merged] = bench.merge([first], [again])
        self.assertEqual(merged["rounds"], [21, 21, 21])
        self.assertEqual(merged["template"]["runs"], 63)
        self.assertEqual(len(merged["reference"]["times_ns"]), 63)
        self.assertFalse(merged["parity"])
        self.assertEqual(bench.merge([first], []), [first])

    def gate(self, rounds: list[list[dict]]) -> tuple[list[dict], list]:
        """`measure` with each round of the bench writing the next of `rounds`."""
        calls = []

        def run(out: Path, only: list[dict] | None = None) -> None:
            calls.append(only)
            entries = rounds[len(calls) - 1]
            for entry in entries:
                entry.pop("rounds", None)  # the bench writes none
            out.write_text("".join(json.dumps(entry) + "\n" for entry in entries), encoding="utf-8")

        with tempfile.TemporaryDirectory() as directory, mock.patch.object(bench, "run", run):
            out = Path(directory) / "templates.jsonl"
            records = bench.measure(out, BUDGET, sequential=True)
            self.assertEqual(bench.load_records(out), records)
        return records, calls

    def test_an_undecided_input_is_measured_until_the_next_look(self) -> None:
        records, calls = self.gate(
            [
                [record(medians=[1.10]), public(medians=[1.20])],
                [record(medians=[1.11]), public(medians=[1.22])],
                [record(medians=[1.09]), public(medians=[1.21])],
                [public(medians=[1.23])],
                [public(medians=[1.21])],
                [public(medians=[1.21])],
            ]
        )
        self.assertIsNone(calls[0])
        self.assertEqual(len(calls[1]), 2)
        # The fixture passed at the look after three rounds; the public
        # model was undecided there and is judged at the next, six rounds,
        # though its interval after five already cleared the ceiling.
        self.assertEqual([entry["input"] for entry in calls[3]], ["bs-sample.ifc"])
        self.assertEqual(len(calls), 6)
        self.assertEqual(records[0]["rounds"], [21, 21, 21])
        self.assertEqual(records[1]["rounds"], [21] * 6)
        self.assertEqual(bench.judge(records, BUDGET, True), [])

    def test_nothing_is_decided_between_looks(self) -> None:
        five = record(medians=[1.20, 1.22, 1.21, 1.23, 1.21])
        self.assertLess(bench.time_judgement(five, BUDGET)["upper"], 1.25)
        self.assertEqual(verdict(five), bench.UNDECIDED)
        self.assertEqual(verdict(record(medians=[0.5, 0.51, 0.49, 0.5])), bench.UNDECIDED)
        # Past the last look (a saved record), it is judged.
        self.assertEqual(verdict(record(medians=[0.5] * 3 + [0.51] * 4)), bench.PASS)

    def test_looks_must_increase_from_two(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "budget.json"
            for looks in ([1, 3], [3, 3], [], "20"):
                path.write_text(json.dumps(dict(BUDGET, looks=looks)), encoding="utf-8")
                with self.assertRaises(SystemExit):
                    bench.load_budget(path)
            path.write_text(json.dumps(BUDGET), encoding="utf-8")
            self.assertEqual(bench.load_budget(path)["looks"], [2, 3, 6])

    def test_an_input_undecided_at_the_bound_fails(self) -> None:
        medians = [1.24, 1.27, 1.23, 1.26, 1.25, 1.24]
        records, calls = self.gate(
            [[record(medians=[1.10]), public(medians=[medians[0]])]]
            + [[record(medians=[1.11]), public(medians=[medians[1]])]]
            + [[record(medians=[1.09]), public(medians=[median])] for median in medians[2:]]
        )
        self.assertEqual(len(calls), 6)
        self.assertEqual(len(records[1]["rounds"]), 6)
        failures = bench.judge(records, BUDGET, True)
        self.assertEqual(len(failures), 1)
        self.assertIn("undecided", failures[0])

    def test_a_round_missing_an_input_stops_the_gate(self) -> None:
        with self.assertRaises(SystemExit):
            self.gate([[record(medians=[1.1]), public(medians=[1.1])], [record(medians=[1.1])]])


if __name__ == "__main__":
    unittest.main()
