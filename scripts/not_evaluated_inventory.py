#!/usr/bin/env python3
"""Rank the causes of not-evaluated outcomes across saved check results.

Usage:
  scripts/not_evaluated_inventory.py [--ruleset R]... [--format F] [--top N]
                                     [LABEL=]RESULT...

Each RESULT is a `result.json` written by `axioval check --report` (with
`--geometry`), one per model; LABEL names that model in the output and
defaults to the file's stem. `--ruleset` maps rule ids to the capability
their definition binds (default: the inventory packages beside this script);
a rule it does not know is listed under its own id.

Every not-evaluated outcome and every unmeasured object is counted once,
under one cause:

- an outcome about an unmeasured object, or whose message names one, is
  caused by that object's unmeasured reason (`unmeasured: ...`);
- any other outcome is caused by its reason code, capability and message
  pattern;
- an unmeasured object no outcome names still counts under its reason.

Messages and reasons are reduced to patterns: object references become
`<object>`, instance ids `#<id>` and numbers `<n>`, and the representation
identifiers a product without a body has (`no body representation; it has
Axis`) `<identifiers>`, so one cause on many objects is one row. An
unmeasured reason that is a fact about the model data (`no shape
representation`: a product with no representation at all; a face whose
boundary crosses or runs back along itself, which the mesh compiler refuses
as `... profile outer ring intersects itself` or `... folds back on itself
at vertex <n>`; a host whose openings remove its whole body; geometry
refused as `#<id> (<TYPE>) is geometrically invalid: ...`; a whole
measured through its parts and unmeasured because a part is, for one of
these) is labelled `unmeasured (model data): ...`. An outcome
refused because a body is not a closed solid (`... is not a closed solid`,
`... neither body is a closed solid`) is model data when every body it
rests on carries the CLI's `shape.open-surface` integrity warning (faces
that, as authored, leave edges bounding one face only): its reason code is
then labelled `<reason> (model data)`. Causes are ranked by the number of not-evaluated
outcomes they account for, then unmeasured objects, then models affected,
then the pattern, so the table is deterministic for the same inputs.

Exit status: 0 on success, 1 when a result cannot be read, 2 for usage.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_RULESET = HERE / "inventory" / "ruleset.json"
DEFAULT_DEFINITIONS = HERE / "inventory" / "definitions.json"

# `ifc-step:model.ifc/#42`, `ifc-step:a.ifczip/b.ifc/#42`: a source-qualified
# object reference as the runtime writes it into messages.
OBJECT_REFERENCE = re.compile(r"[A-Za-z][\w.+-]*:[^\s,;`']*?/#\d+")
INSTANCE = re.compile(r"#\d+")
NUMBER = re.compile(r"(?<![\w<])-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?(?![\w>])")
SPACES = re.compile(r"\s+")
# The identifiers of the representations a product without a body has.
IDENTIFIERS = re.compile(r"(no body representation; it has )[^;]+$")
# Unmeasured reasons that are facts about the model data its author can
# fix, not gaps in the engine or its adapters.
MODEL_DATA = frozenset({"no shape representation"})
# Reason patterns that are model data too: a face ring that crosses or runs
# back along itself bounds no region (#298, the CLI's
# `shape.self-intersecting-face`). A hole joined to the outer boundary by a
# seam is not: it is a valid face, measured on surface paths and refused by
# name in an extruded profile (axiolid/kernel#270).
# So is a host whose openings remove its whole body (#310, the CLI's
# `shape.voided-body`).
# Outcomes refused because a body is no closed solid, and whether the
# subject is one of the bodies (a clash pair) or only the objects named (an
# obstacle in a space's band). Model data when each of those bodies is
# authored open (the CLI's `shape.open-surface`, #311).
OPEN_BODY_OUTCOMES = (
    (re.compile(r"neither body is a closed solid"), True),
    (re.compile(r"(?:reaches below the headroom band|reaches into the band) but is not a "
                r"closed solid"), False),
)
OPEN_SURFACE = "shape.open-surface"
MODEL_DATA_PATTERNS = re.compile(
    r"mesh compilation refused: invalid geometry input: "
    r"(?:planar face|authored polygon face <n>) cannot be triangulated: "
    r"(?:its rings do not bound a region: )?"
    r"profile (?:outer ring|hole <n>) "
    r"(?:intersects itself|folds back on itself at vertex <n>)"
    r"|its openings remove its whole body: nothing is left once the <n> opening\(s\) "
    r"voiding it are subtracted"
    # Geometry `ifc-geometry` refuses as invalid as written (#357, the CLI's
    # `shape.invalid-geometry`): a composite curve with a gap, a boundary
    # point off its plane.
    r"|#<id> \([A-Z0-9_]+\) is geometrically invalid: .+"
)
# A whole measured through its parts and unmeasured through one of them
# (#357): model data when the part's reason is, as the CLI reports it with
# the part's integrity code.
WHOLE_THROUGH_PART = re.compile(
    r"no body representation of its own, and its body is the union of its <n> parts?, "
    r"and part <object> is unmeasured: (?P<part>.+)"
)


def model_data_reason(reason: str) -> bool:
    """Whether an unmeasured reason pattern is a fact about the model data."""
    whole = WHOLE_THROUGH_PART.fullmatch(reason)
    if whole is not None:
        return model_data_reason(whole["part"])
    return reason in MODEL_DATA or MODEL_DATA_PATTERNS.fullmatch(reason) is not None


def pattern(message: str) -> str:
    """The message with object references, instance ids and numbers abstracted."""
    text = OBJECT_REFERENCE.sub("<object>", message)
    text = INSTANCE.sub("#<id>", text)
    text = NUMBER.sub("<n>", text)
    return SPACES.sub(" ", text).strip()


def reason_pattern(reason: str) -> str:
    """An unmeasured reason's pattern, with the identifiers it lists abstracted."""
    return IDENTIFIERS.sub(r"\1<identifiers>", pattern(reason))


def object_key(object_id: dict) -> str:
    """`system:document/#id`, the form the runtime writes into messages."""
    source = object_id.get("source") or {}
    return f"{source.get('system', '')}:{source.get('document', '')}/{object_id.get('local_id', '')}"


def capabilities(rulesets: list[Path], definitions: list[Path]) -> dict[str, str]:
    """Rule id (bare and package-qualified) -> capability id."""
    bound: dict[str, str] = {}
    for path in definitions:
        document = json.loads(path.read_text(encoding="utf-8"))
        for identifier, definition in document.get("definitions", {}).items():
            bound[identifier] = definition.get("capability", identifier)
    mapping: dict[str, str] = {}
    for path in rulesets:
        document = json.loads(path.read_text(encoding="utf-8"))
        package = document.get("package", {}).get("id", "")
        stack = [document.get("root", {})]
        while stack:
            folder = stack.pop()
            for rule in folder.get("rules", []):
                capability = bound.get(rule.get("definitionId", ""), rule.get("definitionId", ""))
                capability = capability.removeprefix("axioval:capability.")
                mapping[rule["id"]] = capability
                mapping[f"{package}/{rule['id']}"] = capability
            stack.extend(folder.get("folders", []))
    return mapping


@dataclass
class Cause:
    kind: str  # "unmeasured" or a reason code
    text: str
    outcomes: int = 0
    objects: int = 0
    models: set[str] = field(default_factory=set)
    capabilities: Counter = field(default_factory=Counter)
    entities: Counter = field(default_factory=Counter)

    def key(self) -> tuple:
        return (-self.outcomes, -self.objects, -len(self.models), self.kind, self.text)

    def model_data(self) -> bool:
        if self.kind != "unmeasured":
            return self.text.startswith(f"{self.kind} (model data) · ")
        return model_data_reason(self.text)

    def label(self) -> str:
        if self.kind != "unmeasured":
            return self.text
        if self.model_data():
            return f"unmeasured (model data): {self.text}"
        return f"unmeasured: {self.text}"


def inventory(results: list[tuple[str, dict]], rules: dict[str, str]) -> list[Cause]:
    causes: dict[tuple[str, str], Cause] = {}

    def cause(kind: str, text: str) -> Cause:
        return causes.setdefault((kind, text), Cause(kind, text))

    for label, result in results:
        objects = result.get("objects") or {}

        def entity(key: str | None) -> str:
            if key is None:
                return "(no object)"
            return (objects.get(key) or {}).get("kind", "(unknown)")

        # Local ids of bodies authored open (one source per result).
        open_bodies = {
            record.get("locator", "").rpartition(":")[2]
            for record in result.get("integrity") or []
            if record.get("code") == OPEN_SURFACE
        }
        unmeasured: dict[str, str] = {}
        for record in (result.get("geometry") or {}).get("unmeasured", []):
            key = object_key(record["object"])
            reason = reason_pattern(record["reason"])
            unmeasured[key] = reason
            found = cause("unmeasured", reason)
            found.objects += 1
            found.models.add(label)
            found.entities[entity(key)] += 1
        for outcome in (result.get("report") or {}).get("not_evaluated", []):
            subject = object_key(outcome["object_id"]) if outcome.get("object_id") else None
            capability = rules.get(outcome["rule_id"], outcome["rule_id"])
            named = sorted(set(OBJECT_REFERENCE.findall(outcome.get("message", ""))))
            blamed = [key for key in ([subject] if subject else []) + named if key in unmeasured]
            if blamed:
                found = cause("unmeasured", unmeasured[blamed[0]])
                blamed_entity = entity(blamed[0])
            else:
                reason = outcome["reason"]
                if authored_open(outcome, named, open_bodies):
                    reason = f"{reason} (model data)"
                text = f"{reason} · {capability} · {pattern(outcome.get('message', ''))}"
                found = cause(outcome["reason"], text)
                blamed_entity = entity(subject)
            found.outcomes += 1
            found.models.add(label)
            found.capabilities[capability] += 1
            if not blamed:
                found.entities[blamed_entity] += 1
    return sorted(causes.values(), key=Cause.key)


def authored_open(outcome: dict, named: list[str], open_bodies: set[str]) -> bool:
    """Whether `outcome` is refused only because bodies authored open are no
    closed solids: every body it rests on carries `shape.open-surface`."""
    message = outcome.get("message", "")
    for expression, subject_too in OPEN_BODY_OUTCOMES:
        if expression.search(message) is None:
            continue
        bodies = [key.rpartition("/")[2] for key in named]
        if subject_too and outcome.get("object_id"):
            bodies.append(outcome["object_id"].get("local_id", ""))
        return bool(bodies) and all(body in open_bodies for body in bodies)
    return False


def top(counter: Counter, limit: int = 3) -> str:
    ranked = sorted(counter.items(), key=lambda item: (-item[1], item[0]))
    shown = ", ".join(f"{name} {count}" for name, count in ranked[:limit])
    rest = len(ranked) - limit
    return shown + (f", +{rest} more" if rest > 0 else "")


def markdown(causes: list[Cause], labels: list[str]) -> str:
    lines = [
        f"Inventory of {len(labels)} model(s): "
        f"{sum(c.outcomes for c in causes)} not-evaluated outcome(s), "
        f"{sum(c.objects for c in causes)} unmeasured object(s), {len(causes)} cause(s).",
        "",
        "| # | Cause | Outcomes | Unmeasured objects | Models | Capabilities | Entities |",
        "|---|---|---:|---:|---:|---|---|",
    ]
    for rank, found in enumerate(causes, 1):
        cell = found.label().replace("|", "\\|")
        lines.append(
            f"| {rank} | {cell} | {found.outcomes} | {found.objects} | {len(found.models)} "
            f"| {top(found.capabilities)} | {top(found.entities)} |"
        )
    return "\n".join(lines) + "\n"


def as_json(causes: list[Cause], labels: list[str]) -> str:
    rows = [
        {
            "rank": rank,
            "cause": found.label(),
            "kind": found.kind,
            "model_data": found.model_data(),
            "outcomes": found.outcomes,
            "unmeasured_objects": found.objects,
            "models": sorted(found.models),
            "capabilities": dict(sorted(found.capabilities.items())),
            "entities": dict(sorted(found.entities.items())),
        }
        for rank, found in enumerate(causes, 1)
    ]
    return json.dumps({"models": labels, "causes": rows}, indent=2, sort_keys=True) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("results", nargs="+", metavar="[LABEL=]RESULT")
    parser.add_argument("--ruleset", action="append", type=Path)
    parser.add_argument("--definitions", action="append", type=Path)
    parser.add_argument("--format", choices=("markdown", "json"), default="markdown")
    parser.add_argument("--top", type=int, default=0, help="rows to print (0: all)")
    arguments = parser.parse_args(argv)

    try:
        rules = capabilities(
            arguments.ruleset or [DEFAULT_RULESET],
            arguments.definitions or [DEFAULT_DEFINITIONS],
        )
        results = []
        for argument in arguments.results:
            label, _, path = argument.rpartition("=")
            path = Path(path)
            results.append((label or path.stem, json.loads(path.read_text(encoding="utf-8"))))
    except (OSError, ValueError, KeyError) as error:
        print(f"not_evaluated_inventory: {error}", file=sys.stderr)
        return 1
    labels = [label for label, _ in results]
    if len(set(labels)) != len(labels):
        print("not_evaluated_inventory: model labels must be distinct", file=sys.stderr)
        return 2

    causes = inventory(results, rules)
    if arguments.top > 0:
        causes = causes[: arguments.top]
    render = as_json if arguments.format == "json" else markdown
    sys.stdout.write(render(causes, labels))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
