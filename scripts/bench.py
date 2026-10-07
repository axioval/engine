#!/usr/bin/env python3
"""Benchmark every capability rebuilt as a template against the
implementation it replaced, and judge it against the recorded budget (#279).

Usage:
  scripts/bench.py gate   [--out FILE]   run, then fail on any budget exceeded
  scripts/bench.py report [--out FILE]   run, then print; fail only if it crashes
  scripts/bench.py judge FILE [--gate]   judge a saved result file

`run` builds and runs `cargo bench -p axioval-cli --bench templates`, which
writes one JSON record per capability and input (see the bench's header),
listing every measured run of both sides in the order they ran.
The budget is `scripts/bench_budget.json`: a template's run time and median
peak heap, each over its reference's, may not exceed `time` and `memory`.

Run time is judged with a stated confidence (#293). Run `i` of the template
and run `i` of the reference ran next to each other, so each pair gives one
ratio, and a round (one process of the bench) its median paired ratio. A
process's layout shifts all its runs alike, so the rounds, not the runs, are
the independent measurements: Student's t interval at `confidence` over the
rounds' (logarithmic) medians decides, at the planned `looks` only (round
counts). A template passes when the interval's upper end is within its
ceiling and fails when its lower end exceeds it. An interval straddling the
ceiling, or a round count between looks, is undecided: a gate measures those
inputs in further rounds, until each is decided or reaches the last look;
one still undecided then fails, since a gate passes only what it shows to be
within budget.

Time is judged per input where the reference's median is at least
`floor_ns`, and over the sum of the medians of the inputs below it; memory
per input. Small inputs may also exceed the summed reference medians by
`small_slack_ns` each, and any input the reference's peak heap by
`slack_bytes`: a fixed cost that small is no regression. A record whose two
sides differ under the parity contract fails,
and so, in a gate, does a public model that was not fetched
(`scripts/parity_models.py fetch`): a gate never passes on less evidence than
it names. Run the gate where nothing else builds (on a shared machine, under
its build lock).

With `GITHUB_STEP_SUMMARY` set, the table is also appended there.
"""

from __future__ import annotations

import json
import math
import os
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BUDGET = ROOT / "scripts" / "bench_budget.json"
DEFAULT_OUT = ROOT / "target" / "bench" / "templates.jsonl"

PASS = "pass"
FAIL = "fail"
UNDECIDED = "undecided"


def load_budget(path: Path = BUDGET) -> dict:
    budget = json.loads(path.read_text(encoding="utf-8"))
    for key in ("time", "memory", "floor_ns", "small_slack_ns", "slack_bytes", "confidence"):
        value = budget.get(key)
        if not isinstance(value, (int, float)) or value <= 0:
            raise SystemExit(f"{path}: `{key}` must be a positive number")
    if budget["confidence"] >= 1:
        raise SystemExit(f"{path}: `confidence` must be below 1")
    looks = budget.get("looks")
    if (
        not isinstance(looks, list)
        or not looks
        or not all(isinstance(look, int) for look in looks)
        or looks[0] < 2
        or any(later <= earlier for earlier, later in zip(looks, looks[1:]))
    ):
        raise SystemExit(f"{path}: `looks` lists increasing round counts, the first at least 2")
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


def _incomplete_beta(x: float, a: float, b: float) -> float:
    """The regularized incomplete beta function `I_x(a, b)`, by its
    continued fraction (Lentz's method)."""
    if x <= 0:
        return 0.0
    if x >= 1:
        return 1.0
    if x > (a + 1) / (a + b + 2):
        return 1.0 - _incomplete_beta(1 - x, b, a)
    front = math.exp(math.lgamma(a + b) - math.lgamma(a) - math.lgamma(b) + a * math.log(x) + b * math.log(1 - x)) / a
    tiny = 1e-300
    c, d = 1.0, 1.0 - (a + b) * x / (a + 1)
    d = 1.0 / (d if abs(d) > tiny else tiny)
    result = d
    for m in range(1, 300):
        for numerator in (
            m * (b - m) * x / ((a + 2 * m - 1) * (a + 2 * m)),
            -(a + m) * (a + b + m) * x / ((a + 2 * m) * (a + 2 * m + 1)),
        ):
            d = 1.0 + numerator * d
            d = 1.0 / (d if abs(d) > tiny else tiny)
            c = 1.0 + numerator / c
            c = c if abs(c) > tiny else tiny
            result *= c * d
        if abs(c * d - 1.0) < 1e-15:
            break
    return front * result


def student_quantile(probability: float, df: int) -> float:
    """The `probability` quantile (above one half) of Student's t
    distribution with `df` degrees of freedom, by bisection of its CDF."""
    def cdf(t: float) -> float:
        return 1.0 - 0.5 * _incomplete_beta(df / (df + t * t), df / 2, 0.5)

    low, high = 0.0, 1.0
    while cdf(high) < probability:
        high *= 2
    for _ in range(200):
        middle = (low + high) / 2
        if cdf(middle) < probability:
            low = middle
        else:
            high = middle
    return (low + high) / 2


def paired_ratios(record: dict) -> list[float]:
    """The template's run time over the reference's, run by run (runs `i`
    of both sides ran next to each other). A record without its runs
    (saved before #293) gives its ratio of medians alone."""
    template = record["template"].get("times_ns")
    reference = record["reference"].get("times_ns")
    if not template or not reference or len(template) != len(reference):
        return [record["time_ratio"]]
    return [t / max(r, 1) for t, r in zip(template, reference)]


def round_medians(record: dict) -> list[float]:
    """Each round's median paired ratio, as a logarithm. A round is one
    process of the bench; `rounds` lists their runs in order, and a record
    without it is one round."""
    ratios = paired_ratios(record)
    sizes = record.get("rounds") or [len(ratios)]
    if sum(sizes) != len(ratios):
        raise SystemExit(f"{record['capability']} on {record['input']}: `rounds` does not count its runs")
    medians, start = [], 0
    for size in sizes:
        medians.append(statistics.median(math.log(ratio) for ratio in ratios[start : start + size]))
        start += size
    return medians


def time_judgement(record: dict, budget: dict) -> dict:
    """The run-time verdict of one input above the floor. Each round gives
    its median paired ratio; their geometric mean is the ratio, and
    Student's t interval over the rounds at the budget's confidence decides
    pass (its upper end within the input's ceiling), fail (its lower end
    over it) or undecided. It decides only at a planned look (a round count
    in `looks`, or past the last), so measuring until an interval happens
    to clear the ceiling cannot pass a template. One round gives no
    interval."""
    medians = round_medians(record)
    allowed = allowance(budget, record)["time"]
    rounds = len(medians)
    centre = statistics.fmean(medians)
    if rounds < 2:
        spread, lower, upper = math.nan, 0.0, math.inf
    else:
        spread = statistics.stdev(medians)
        width = student_quantile((1 + budget["confidence"]) / 2, rounds - 1) * spread / math.sqrt(rounds)
        lower, upper = math.exp(centre - width), math.exp(centre + width)
    looks = budget["looks"]
    if rounds not in looks and rounds < looks[-1]:
        verdict = UNDECIDED
    elif upper <= allowed:
        verdict = PASS
    elif lower > allowed:
        verdict = FAIL
    else:
        verdict = UNDECIDED
    return {
        "ratio": math.exp(centre),
        "lower": lower,
        "upper": upper,
        "spread": spread,
        "rounds": rounds,
        "runs": len(paired_ratios(record)),
        "allowed": allowed,
        "verdict": verdict,
    }


def above_floor(record: dict, budget: dict) -> bool:
    return not record.get("missing") and record["reference"]["median_ns"] >= budget["floor_ns"]


def undecided(records: list[dict], budget: dict) -> list[dict]:
    """The records above the floor whose run time is undecided before the
    last look: the ones further rounds may decide."""
    return [
        record
        for record in records
        if above_floor(record, budget)
        and time_judgement(record, budget)["verdict"] == UNDECIDED
        and len(round_medians(record)) < budget["looks"][-1]
    ]


def selected(records: list[dict], named: list[dict]) -> list[dict]:
    """The records of `records` for the inputs `named` lists."""
    keys = {(record["capability"], record["input"]) for record in named}
    return [record for record in records if (record.get("capability"), record.get("input")) in keys]


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
        heap = record["template"].get("peak_bytes")
        within_slack = heap is not None and heap <= record["reference"].get("peak_bytes", 0) + budget["slack_bytes"]
        if record["memory_ratio"] > allowed["memory"] and not within_slack:
            failures.append(
                f"{name}: peak heap {record['memory_ratio']:.2f}x the reference's "
                f"exceeds {allowed['memory']}x"
            )
        if above_floor(record, budget):
            time = time_judgement(record, budget)
            interval = (
                f"run time {time['ratio']:.3f}x the reference's "
                f"({budget['confidence']:.0%} interval {time['lower']:.3f} to {time['upper']:.3f} "
                f"over {time['rounds']} rounds)"
            )
            if time["verdict"] == FAIL:
                failures.append(f"{name}: {interval} exceeds {time['allowed']}x")
            elif time["verdict"] == UNDECIDED:
                failures.append(f"{name}: {interval} is undecided against {time['allowed']}x")
        else:
            small.setdefault(record["capability"], []).append(record)
    for capability, group in small.items():
        template = sum(record["template"]["median_ns"] for record in group)
        reference = sum(record["reference"]["median_ns"] for record in group)
        ratio = template / max(reference, 1)
        slack = budget["small_slack_ns"] * len(group)
        if ratio > budget["time"] and template > reference + slack:
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


def merge(records: list[dict], again: list[dict]) -> list[dict]:
    """`records` with the runs of `again` (the same inputs measured in
    another round) pooled into theirs: run lists concatenated, the round's
    runs counted in `rounds`, medians and ratios recomputed over all of them,
    parity holding only if it held each time."""
    more = {(record["capability"], record["input"]): record for record in again}
    merged = []
    for record in records:
        extra = more.get((record.get("capability"), record.get("input")))
        if extra is None or record.get("missing") or extra.get("missing"):
            merged.append(record)
            continue
        sizes = record.get("rounds") or [len(paired_ratios(record))]
        record = dict(record)
        record["rounds"] = sizes + (extra.get("rounds") or [len(paired_ratios(extra))])
        for side in ("template", "reference"):
            runs = dict(record[side])
            runs["times_ns"] = runs.get("times_ns", []) + extra[side].get("times_ns", [])
            runs["peaks_bytes"] = runs.get("peaks_bytes", []) + extra[side].get("peaks_bytes", [])
            times, peaks = sorted(runs["times_ns"]), sorted(runs["peaks_bytes"])
            runs.update(
                runs=len(times),
                median_ns=times[len(times) // 2],
                min_ns=times[0],
                max_ns=times[-1],
                peak_bytes=peaks[len(peaks) // 2],
            )
            record[side] = runs
        record["parity"] = bool(record.get("parity")) and bool(extra.get("parity"))
        record["differences"] = record.get("differences", []) + extra.get("differences", [])
        record["time_ratio"] = record["template"]["median_ns"] / max(record["reference"]["median_ns"], 1)
        record["memory_ratio"] = record["template"]["peak_bytes"] / max(record["reference"]["peak_bytes"], 1)
        merged.append(record)
    return merged


def table(records: list[dict], budget: dict) -> str:
    rows = [
        "| capability | input | objects | template | reference | time | interval | spread | rounds | verdict | peak heap | parity |",
        "| --- | --- | ---: | ---: | ---: | ---: | --- | ---: | ---: | --- | ---: | --- |",
    ]
    for record in records:
        capability = record["capability"].removeprefix("axioval:capability.")
        if record.get("missing"):
            rows.append(f"| {capability} | {record['input']} | | | | | | | | | | not fetched |")
            continue
        if above_floor(record, budget):
            time = time_judgement(record, budget)
            ratio = f"{time['ratio']:.2f}x"
            interval = f"{time['lower']:.2f} to {time['upper']:.2f}" if time["rounds"] > 1 else ""
            spread = f"{time['spread']:.1%}" if time["rounds"] > 1 else ""
            verdict = time["verdict"]
        else:
            ratio, interval, spread, verdict = f"{record['time_ratio']:.2f}x", "", "", "under the floor"
        rows.append(
            f"| {capability} | {record['input']} | {record['objects']} "
            f"| {record['template']['median_ns'] / 1000:.1f} µs "
            f"| {record['reference']['median_ns'] / 1000:.1f} µs "
            f"| {ratio} | {interval} | {spread} | {len(round_medians(record))} | {verdict} "
            f"| {record['memory_ratio']:.2f}x "
            f"| {'holds' if record['parity'] else 'differs'} |"
        )
    return "\n".join(rows) + "\n"


def run(out: Path, only: list[dict] | None = None) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    command = ["cargo", "bench", "-p", "axioval-cli", "--bench", "templates", "--", "--out", str(out)]
    if only is not None:
        selection = out.with_suffix(".only.json")
        selection.write_text(json.dumps([[record["capability"], record["input"]] for record in only]), encoding="utf-8")
        command += ["--only", str(selection)]
    status = subprocess.run(command, cwd=ROOT, check=False).returncode
    if status != 0:
        raise SystemExit(status)


def measure(out: Path, budget: dict, sequential: bool) -> list[dict]:
    """Measures every input in one round (one process of the bench), then,
    if `sequential`, the inputs above the floor still undecided in another
    round each, until each is decided or has the last look's rounds; the pooled
    records are written back to `out`, so `judge` reproduces the verdict."""
    run(out)
    records = load_records(out)
    while sequential and (pending := undecided(records, budget)):
        names = ", ".join(f"{record['capability']} on {record['input']}" for record in pending[:5])
        more = f" and {len(pending) - 5} more" if len(pending) > 5 else ""
        print(f"run time undecided; another round of {names}{more}", file=sys.stderr)
        again = out.with_suffix(".again.jsonl")
        run(again, pending)
        before = sum(len(round_medians(record)) for record in pending)
        records = merge(records, load_records(again))
        if sum(len(round_medians(record)) for record in selected(records, pending)) < before + len(pending):
            raise SystemExit("a round did not measure every input it was asked for")
        out.write_text("".join(json.dumps(record) + "\n" for record in records), encoding="utf-8")
    return records


def verdict(records: list[dict], gate: bool) -> int:
    budget = load_budget()
    rendered = table(records, budget)
    failures = judge(records, budget, gate)
    lines = [
        f"budget: {budget['time']}x run time ({budget['confidence']:.0%} interval over rounds, "
        f"judged after {', '.join(map(str, budget['looks']))} rounds), {budget['memory']}x peak heap"
    ]
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
    records = measure(out, load_budget(), sequential=gate)
    return verdict(records, gate)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
