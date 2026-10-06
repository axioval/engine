#!/usr/bin/env python3
"""Benchmark every capability rebuilt as a template against the
implementation it replaced, and judge it against the recorded budget (#279).

Usage:
  scripts/bench.py gate   [--out FILE]   run, then fail on any budget exceeded
  scripts/bench.py report [--out FILE]   run, then print; fail only if it crashes
  scripts/bench.py judge FILE [--gate]   judge a saved result file

`run` builds and runs `cargo bench -p axioval-cli --bench templates`, which
writes one JSON record per capability and input (see the bench's header).
The budget is `scripts/bench_budget.json`: a template's median run time and
median peak heap, each over its reference's, may not exceed `time` and
`memory`. Time is judged per input where the reference's median is at least
`floor_ns`, and over the sum of the medians of the inputs below it; memory
per input. A record whose two sides differ under the parity contract fails,
and so, in a gate, does a public model that was not fetched
(`scripts/parity_models.py fetch`): a gate never passes on less evidence than
it names.

A gate whose only failures are run times over budget measures once more and
judges that measurement, since a loaded machine slows a run; a real regression
fails both. Run the gate where nothing else builds (on a shared machine, under
its build lock).

With `GITHUB_STEP_SUMMARY` set, the table is also appended there.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BUDGET = ROOT / "scripts" / "bench_budget.json"
DEFAULT_OUT = ROOT / "target" / "bench" / "templates.jsonl"


def load_budget(path: Path = BUDGET) -> dict:
    budget = json.loads(path.read_text(encoding="utf-8"))
    for key in ("time", "memory", "floor_ns"):
        value = budget.get(key)
        if not isinstance(value, (int, float)) or value <= 0:
            raise SystemExit(f"{path}: `{key}` must be a positive number")
    for exception in budget.get("exceptions", []):
        if not exception.get("reason") or not exception.get("capability") or not exception.get("input"):
            raise SystemExit(f"{path}: an exception names its capability, input and reason")
    return budget


def load_records(path: Path) -> list[dict]:
    lines = path.read_text(encoding="utf-8").splitlines()
    return [json.loads(line) for line in lines if line.strip()]


def allowance(budget: dict, record: dict) -> dict:
    """The ceilings `record` is held to: the budget's, or a recorded
    exception's for its capability and input."""
    for exception in budget.get("exceptions", []):
        if (exception["capability"], exception["input"]) == (record["capability"], record["input"]):
            return {
                "time": exception.get("time", budget["time"]),
                "memory": exception.get("memory", budget["memory"]),
            }
    return {"time": budget["time"], "memory": budget["memory"]}


def judge(records: list[dict], budget: dict, gate: bool) -> list[str]:
    """Every way `records` break `budget`, one line each; none means it holds."""
    failures: list[str] = []
    if not records:
        return ["the benchmark measured nothing"]
    small: dict[str, list[dict]] = {}
    for record in records:
        name = f"{record['capability']} on {record['input']}"
        if record.get("missing"):
            if gate:
                failures.append(f"{name}: the public model is not fetched")
            continue
        if not record.get("parity", False):
            failures.append(f"{name}: template and reference differ under the parity contract")
        allowed = allowance(budget, record)
        if record["memory_ratio"] > allowed["memory"]:
            failures.append(
                f"{name}: peak heap {record['memory_ratio']:.2f}x the reference's "
                f"exceeds {allowed['memory']}x"
            )
        if record["reference"]["median_ns"] >= budget["floor_ns"]:
            if record["time_ratio"] > allowed["time"]:
                failures.append(
                    f"{name}: run time {record['time_ratio']:.2f}x the reference's "
                    f"exceeds {allowed['time']}x"
                )
        else:
            small.setdefault(record["capability"], []).append(record)
    for capability, group in small.items():
        template = sum(record["template"]["median_ns"] for record in group)
        reference = sum(record["reference"]["median_ns"] for record in group)
        ratio = template / max(reference, 1)
        if ratio > budget["time"]:
            failures.append(
                f"{capability} on {len(group)} inputs under the floor: run time "
                f"{ratio:.2f}x the reference's exceeds {budget['time']}x"
            )
    measured = [record for record in records if not record.get("missing")]
    if gate and not any(record["input"].startswith("fixture-") for record in measured):
        failures.append("no fixture was measured")
    if gate and not any(not record["input"].startswith("fixture-") for record in measured):
        failures.append("no public model was measured")
    return failures


def table(records: list[dict]) -> str:
    rows = [
        "| capability | input | objects | template | reference | time | peak heap | parity |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |",
    ]
    for record in records:
        capability = record["capability"].removeprefix("axioval:capability.")
        if record.get("missing"):
            rows.append(f"| {capability} | {record['input']} | | | | | | not fetched |")
            continue
        rows.append(
            f"| {capability} | {record['input']} | {record['objects']} "
            f"| {record['template']['median_ns'] / 1000:.1f} µs "
            f"| {record['reference']['median_ns'] / 1000:.1f} µs "
            f"| {record['time_ratio']:.2f}x "
            f"| {record['memory_ratio']:.2f}x "
            f"| {'holds' if record['parity'] else 'differs'} |"
        )
    return "\n".join(rows) + "\n"


def run(out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    command = ["cargo", "bench", "-p", "axioval-cli", "--bench", "templates", "--", "--out", str(out)]
    status = subprocess.run(command, cwd=ROOT, check=False).returncode
    if status != 0:
        raise SystemExit(status)


def verdict(records: list[dict], gate: bool) -> int:
    budget = load_budget()
    rendered = table(records)
    failures = judge(records, budget, gate)
    lines = [f"budget: {budget['time']}x run time, {budget['memory']}x peak heap"]
    lines += [f"FAIL {failure}" for failure in failures] or ["every template is within its budget"]
    text = rendered + "\n" + "\n".join(lines) + "\n"
    print(text, end="")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write("## Template benchmark\n\n" + text)
    return 1 if gate and failures else 0


def main(argv: list[str]) -> int:
    if not argv or argv[0] not in ("gate", "report", "judge"):
        print(__doc__, file=sys.stderr)
        return 2
    command, rest = argv[0], argv[1:]
    if command == "judge":
        if not rest:
            print("judge needs a result file", file=sys.stderr)
            return 2
        return verdict(load_records(Path(rest[0])), gate="--gate" in rest[1:])
    out = DEFAULT_OUT
    if "--out" in rest:
        index = rest.index("--out")
        if index + 1 >= len(rest):
            print("--out needs a file", file=sys.stderr)
            return 2
        out = Path(rest[index + 1]).resolve()
    gate = command == "gate"
    run(out)
    records = load_records(out)
    if gate and retimed(judge(records, load_budget(), gate)):
        # A run time over budget alone may be the machine's load: measure
        # once more and judge that. A real regression fails both.
        print("run time over budget; measuring once more", file=sys.stderr)
        run(out)
        records = load_records(out)
    return verdict(records, gate)


def retimed(failures: list[str]) -> bool:
    """Whether every failure is a run time over budget, which one more
    measurement may clear; any other failure stands as measured."""
    return bool(failures) and all("run time" in failure for failure in failures)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
