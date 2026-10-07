"""Composability gate (#225, #274): every measurement a capability takes is
exposed as a registered measured value, call site by call site.

Measurement in a measured value, decision in an expression or a generic
judge, search-only logic in a capability. The gate binds three sources:

- The service traits. Every method of a `pub trait …Service` in a core
  crate carries a `// gate:` marker: `// gate: measures <kinds>` when it
  returns a measured quantity, `// gate: reads` when it returns stated source
  facts, identities, relations or configuration. A method named `measure_*`
  cannot be marked `reads`. The set of measuring methods is read from these
  markers, so renaming a method does not hide it, and a new method without a
  marker fails.
- The rules crate. Every call of a measuring method (by name and arity, or
  as a `…Service::method` path), and every inline geometry primitive
  (`hypot`, `sqrt`, `atan2`, trigonometry), in a file that is not a
  registered measured-value provider is a call site, keyed by
  `module::function::method` and counted.
- `scripts/measurement_ledger.json`. Each call site key has exactly one
  entry with the counted number of calls, mapping it to the registered
  measured values that provide it (`providedBy`), or to a reviewed search
  kind from `SEARCH_KINDS` with a reason. A value named in `providedBy` must
  be in the authoring catalogue, list the called method's service, and be of
  a kind the method's marker declares (a plain number, a count or a share,
  may be taken from any). `unregistered` entries name the issue registering
  them and stay within `UNREGISTERED_BUDGET`.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

SERVICE_TRAIT = re.compile(r"pub\s+trait\s+(\w*Service)\b[^{]*\{", re.M)
FN_NAME = re.compile(r"\bfn\s+(\w+)")
MARKER = re.compile(r"//\s*gate:\s*(.*?)\s*$")
CAMEL_BOUNDARY = re.compile(r"(?<!^)(?=[A-Z])")

# The kinds a measuring method may yield: a value's dimension in the
# catalogue (`number` for a plain number), or `truth` for a decided fact.
QUANTITY_KINDS = frozenset({"length", "area", "volume", "plane_angle", "number", "truth"})

# Inline geometry in a capability: computing a distance, angle or root itself
# rather than taking it from a service.
INLINE_PRIMITIVES = ("hypot", "sqrt", "atan2", "atan", "acos", "asin", "sin", "cos", "tan")
INLINE_CALL = re.compile(r"(?:\.|\bf64::)(" + "|".join(INLINE_PRIMITIVES) + r")\s*\(")

# The closed vocabulary of reviewed reasons a call site is no measurement
# a rule could state as an expression. Adding a kind is a change to this
# gate, reviewed as such; a ledger entry cannot invent one.
SEARCH_KINDS: dict[str, str] = {
    "candidate-filter": (
        "the quantity only narrows which objects are compared; the decision is "
        "another service's or another measured value's"
    ),
    "pairing": "the quantity only matches an object with its counterpart",
    "route-search": (
        "a path search; the route's own quantities are the stair, ramp and "
        "travel values"
    ),
    "obstruction-search": "searches a zone or sight line for obstructing objects",
    "description": "describes a finding already decided, never decides it",
    "classification": "assigns an object to a container, side or envelope class",
    "revision-comparison": (
        "compares an object with its counterpart in another revision; a "
        "difference between two models is no value of one object"
    ),
    "derivation": (
        "inline arithmetic on values already measured or stated (a unit, "
        "threshold or parameter conversion); measures nothing new"
    ),
    "unregistered": (
        "a measurement no registered value exposes yet; the reason names the "
        "issue that registers it, and the budget below only falls"
    ),
}
# A service call is a measurement; only inline arithmetic can be a derivation.
INLINE_ONLY_KINDS = frozenset({"derivation"})
MIN_REASON_WORDS = 4
# Known measurements still to be registered as values. Lower it as each is
# registered; it never rises.
UNREGISTERED_BUDGET = 0
ISSUE_REFERENCE = re.compile(r"#\d+\b")

PROVIDER_IMPL = re.compile(r"\bimpl\s+MeasuredProvider\s+for\s+(\w+)")
CAPABILITY_IMPL = re.compile(r"\bimpl\s+RuleCapability\s+for\b")
REGISTERED_PROVIDER = re.compile(r"register_measured\(\s*(?:\w+::)*(\w+)\s*\)")
QUALIFIED_CALL = re.compile(r"\b\w*Service(?:Handle)?\s*>?\s*::\s*(\w+)\b")


NOISE = re.compile(
    r"//[^\n]*"  # a line or doc comment
    r"|/\*.*?\*/"  # a block comment
    r"|\br(#*)\".*?\"\1"  # a raw string
    r"|\"(?:\\.|[^\"\\])*\""  # a string
    r"|'(?:\\.|[^\\'\n])'",  # a char literal, never a lifetime
    re.S,
)


def strip_noise(source: str) -> str:
    """Blank comments and string and char literals, preserving offsets."""
    return NOISE.sub(lambda match: blank(match.group(0)), source)


def blank(text: str) -> str:
    return "\n".join(" " * len(line) for line in text.split("\n"))


BRACKETS = {pair: re.compile(re.escape(pair[0]) + "|" + re.escape(pair[1])) for pair in ("()", "{}")}


def matching(text: str, open_index: int, opening: str, closing: str) -> int:
    """The index just past the bracket closing the one at `open_index`."""
    depth = 1
    for match in BRACKETS[opening + closing].finditer(text, open_index + 1):
        depth += 1 if match.group(0) == opening else -1
        if not depth:
            return match.end()
    return len(text)


def item_stop(scan: str, index: int, end: int) -> int:
    """The `{` opening a body or the `;` ending a declaration after `index`.

    Brackets are skipped, so a return type such as `[f64; 2]` does not end
    the declaration early.
    """
    depth = 0
    while index < end:
        char = scan[index]
        if char in "([":
            depth += 1
        elif char in ")]":
            depth -= 1
        elif char in "{;" and depth == 0:
            return index
        index += 1
    return end


def top_level_arguments(inner: str, angles: bool = False) -> int:
    """How many comma-separated arguments `inner` holds at depth zero."""
    if not inner.strip():
        return 0
    depth, count = 0, 1
    previous = ""
    for char in inner:
        if char in "([{" or (angles and char == "<"):
            depth += 1
        elif char in ")]}" or (angles and char == ">" and previous != "-"):
            depth -= 1
        elif char == "," and depth == 0:
            count += 1
        previous = char
    if inner.rstrip().endswith(","):
        count -= 1
    return count


def service_name(trait: str) -> str:
    """`WalkingSurfaceService` is the service `walking-surface`."""
    return CAMEL_BOUNDARY.sub("-", trait.removesuffix("Service")).lower()


def marker_above(source: str, offset: int) -> str | None:
    """The `// gate:` marker among the comment and attribute lines above."""
    lines = source[:offset].split("\n")
    lines.pop()  # the `fn` line itself, up to the name
    found = None
    while lines:
        line = lines.pop().strip()
        if not (line.startswith("//") or line.startswith("#[")):
            break
        match = MARKER.match(line)
        if match and found is None:
            found = match.group(1)
    return found


def service_methods(source: str) -> tuple[list[dict], list[str]]:
    """Each service trait method with its marker, and marker violations."""
    scan = strip_noise(source)
    methods: list[dict] = []
    failures: list[str] = []
    for trait in SERVICE_TRAIT.finditer(scan):
        end = matching(scan, trait.end() - 1, "{", "}")
        body_start = trait.end()
        index = body_start
        while True:
            match = FN_NAME.search(scan, index, end)
            if not match:
                break
            name = match.group(1)
            open_paren = scan.find("(", match.end())
            close = matching(scan, open_paren, "(", ")")
            arity = max(top_level_arguments(scan[open_paren + 1 : close - 1], angles=True) - 1, 0)
            # Skip a default body so its inner functions are not methods.
            stop = item_stop(scan, close, end)
            index = matching(scan, stop, "{", "}") if scan[stop : stop + 1] == "{" else stop + 1
            where = f"service `{trait.group(1)}::{name}`"
            marker = marker_above(source, match.start())
            if marker is None:
                failures.append(
                    f"{where} has no `// gate: measures <kinds>` or `// gate: reads` marker"
                )
                continue
            words = marker.replace(",", " ").split()
            if words == ["reads"]:
                if name.startswith("measure_"):
                    failures.append(f"{where} is named as a measurement but marked `reads`")
                kinds: frozenset[str] = frozenset()
            elif words[:1] == ["measures"] and len(words) > 1:
                kinds = frozenset(words[1:])
                unknown = sorted(kinds - QUANTITY_KINDS)
                if unknown:
                    failures.append(f"{where} declares unknown kinds {', '.join(unknown)}")
            else:
                failures.append(f"{where} has a malformed marker `// gate: {marker}`")
                continue
            methods.append(
                {
                    "trait": trait.group(1),
                    "service": service_name(trait.group(1)),
                    "method": name,
                    "arity": arity,
                    "kinds": kinds,
                }
            )
    return methods, failures


def measuring_methods(methods: list[dict]) -> dict[str, list[dict]]:
    """Measuring methods by name: one name may be declared by several traits."""
    by_name: dict[str, list[dict]] = {}
    for method in methods:
        if method["kinds"]:
            by_name.setdefault(method["method"], []).append(method)
    return by_name


def enclosing_functions(scan: str) -> list[tuple[int, int, str]]:
    """(start, end, name) of every function body, innermost found last."""
    spans: list[tuple[int, int, str]] = []
    for match in FN_NAME.finditer(scan):
        open_paren = scan.find("(", match.end())
        if open_paren < 0:
            continue
        close = matching(scan, open_paren, "(", ")")
        start = item_stop(scan, close, len(scan))
        if scan[start : start + 1] != "{":
            continue
        spans.append((start, matching(scan, start, "{", "}"), match.group(1)))
    return spans


def function_at(spans: list[tuple[int, int, str]], offset: int) -> str:
    inner = [span for span in spans if span[0] <= offset < span[1]]
    if not inner:
        return "<item>"
    return max(inner, key=lambda span: span[0])[2]


def module_path(path: str) -> str:
    """`opening_zone/mod.rs` is `opening_zone`; `a/b.rs` is `a::b`."""
    parts = path.removesuffix(".rs").split("/")
    if parts[-1] == "mod":
        parts.pop()
    return "::".join(parts)


def provider_files(files: dict[str, str], lib: str) -> set[str]:
    """Files implementing only registered measured-value providers."""
    registered = set(REGISTERED_PROVIDER.findall(lib))
    providers: set[str] = set()
    for path, text in files.items():
        code = strip_noise(text)
        implemented = PROVIDER_IMPL.findall(code)
        if implemented and set(implemented) <= registered and not CAPABILITY_IMPL.search(code):
            providers.add(path)
    return providers


def call_sites(
    files: dict[str, str], measuring: dict[str, list[dict]], exempt: set[str]
) -> dict[str, dict]:
    """Every measuring call and inline primitive, keyed and counted."""
    sites: dict[str, dict] = {}

    def record(path: str, spans, offset: int, method: str, declared: list[dict] | None) -> None:
        key = f"{module_path(path)}::{function_at(spans, offset)}::{method}"
        site = sites.setdefault(key, {"count": 0, "declared": declared})
        site["count"] += 1

    for path, text in sorted(files.items()):
        if path in exempt:
            continue
        scan = strip_noise(text)
        spans = enclosing_functions(scan)
        for match in re.finditer(r"\.\s*(\w+)\s*(?:::\s*<[^>]*>\s*)?\(", scan):
            name = match.group(1)
            if name not in measuring:
                continue
            open_paren = match.end() - 1
            close = matching(scan, open_paren, "(", ")")
            arity = top_level_arguments(scan[open_paren + 1 : close - 1])
            declared = [m for m in measuring[name] if m["arity"] == arity]
            if declared:
                record(path, spans, match.start(), name, declared)
        for match in QUALIFIED_CALL.finditer(scan):
            name = match.group(1)
            if name in measuring:
                record(path, spans, match.start(), name, measuring[name])
        for match in INLINE_CALL.finditer(scan):
            record(path, spans, match.start(), match.group(1), None)
    return sites


def catalogue_values(catalogue: dict) -> dict[str, dict]:
    """Registered measured values and member lists: their services and kinds."""
    values: dict[str, dict] = {}
    for value in catalogue.get("measuredValues", []):
        if value.get("available"):
            values[value["name"]] = {
                "services": set(value["services"]),
                "kinds": {value.get("dimension") or "number"},
            }
    # A list states which members exist (a truth per member) and the fields
    # each member holds.
    for members in catalogue.get("measuredMembers", []):
        if members.get("available"):
            kinds = {"truth"} | {
                "truth" if field["kind"]["type"] == "truth"
                else field["kind"].get("dimension") or "number"
                for field in members["fields"]
            }
            values[members["name"]] = {"services": set(members["services"]), "kinds": kinds}
    return values


def ledger_violations(sites: dict[str, dict], ledger: dict, values: dict[str, dict]) -> list[str]:
    """Call sites the ledger does not map to a provider or a reviewed search."""
    failures: list[str] = []
    entries: dict[str, dict] = ledger.get("callSites", {})
    unregistered = sorted(
        key for key, entry in entries.items() if entry.get("searchOnly") == "unregistered"
    )
    if len(unregistered) > UNREGISTERED_BUDGET:
        failures.append(
            f"{len(unregistered)} call sites are unregistered measurements, over the budget of "
            f"{UNREGISTERED_BUDGET}: register a measured value instead"
        )
    for key in sorted(set(sites) - set(entries)):
        what = "inline geometry" if sites[key]["declared"] is None else "a measuring service"
        failures.append(
            f"call site `{key}` takes {what} ({sites[key]['count']}x) with no entry in "
            "scripts/measurement_ledger.json; map it to the measured value that provides "
            "it, or register one"
        )
    for key in sorted(set(entries) - set(sites)):
        failures.append(f"ledger entry `{key}` is stale: no such call site")
    for key in sorted(set(entries) & set(sites)):
        entry, site = entries[key], sites[key]
        if entry.get("count") != site["count"]:
            failures.append(
                f"call site `{key}` is taken {site['count']}x but the ledger counts "
                f"{entry.get('count')}; map the new call"
            )
        provided = entry.get("providedBy")
        search = entry.get("searchOnly")
        if (provided is None) == (search is None):
            failures.append(f"ledger entry `{key}` needs exactly one of providedBy, searchOnly")
            continue
        if search is not None:
            if search not in SEARCH_KINDS:
                failures.append(
                    f"ledger entry `{key}` names search kind `{search}`, not one of "
                    f"{', '.join(sorted(SEARCH_KINDS))}"
                )
            elif search in INLINE_ONLY_KINDS and site["declared"] is not None:
                failures.append(f"ledger entry `{key}`: a service measurement is no `{search}`")
            reason = str(entry.get("reason", ""))
            if len(reason.split()) < MIN_REASON_WORDS:
                failures.append(f"ledger entry `{key}` gives no reviewed reason it only searches")
            elif search == "unregistered" and not ISSUE_REFERENCE.search(reason):
                failures.append(f"ledger entry `{key}` names no issue that registers it")
            continue
        if not provided:
            failures.append(f"ledger entry `{key}` names no measured value")
        declared = site["declared"]
        for name in provided:
            value = values.get(name)
            if value is None:
                failures.append(
                    f"ledger entry `{key}` names `{name}`, which is no registered measured value"
                )
                continue
            if declared is None:
                continue
            services = {method["service"] for method in declared}
            if not value["services"] & services:
                failures.append(
                    f"ledger entry `{key}` names `{name}`, which is not measured through "
                    f"{', '.join(sorted(services))}"
                )
            # A plain number (a count or a share) may be taken from any
            # quantity the method measures; a dimension must be one it declares.
            kinds = set().union(*(method["kinds"] for method in declared))
            if "number" not in value["kinds"] and not value["kinds"] & kinds:
                failures.append(
                    f"ledger entry `{key}` names `{name}` ({', '.join(sorted(value['kinds']))}), "
                    f"but the call measures {', '.join(sorted(kinds))}"
                )
    return failures


def violations(
    trait_sources: list[tuple[str, str]],
    files: dict[str, str],
    ledger: dict,
    catalogue: dict,
) -> list[str]:
    """The whole composability check over in-memory sources."""
    methods: list[dict] = []
    failures: list[str] = []
    for path, source in trait_sources:
        found, marker_failures = service_methods(source)
        methods.extend(found)
        failures.extend(f"{path}: {failure}" for failure in marker_failures)
    measuring = measuring_methods(methods)
    if not measuring:
        failures.append("no service method is marked measuring; the gate would be inert")
    sites = call_sites(files, measuring, provider_files(files, files.get("lib.rs", "")))
    failures.extend(ledger_violations(sites, ledger, catalogue_values(catalogue)))
    return failures


def check(root: Path, core_roots: list[Path]) -> list[str]:
    rules = root / "crates" / "engine" / "rules"
    files = {
        source.relative_to(rules / "src").as_posix(): source.read_text(encoding="utf-8")
        for source in sorted((rules / "src").rglob("*.rs"))
    }
    trait_sources = [
        (source.relative_to(root).as_posix(), source.read_text(encoding="utf-8"))
        for crate in core_roots
        for source in sorted((crate / "src").rglob("*.rs"))
    ]
    ledger = json.loads((root / "scripts" / "measurement_ledger.json").read_text(encoding="utf-8"))
    catalogue = json.loads(
        (rules / "tests" / "golden" / "catalogue.json").read_text(encoding="utf-8")
    )
    return violations(trait_sources, files, ledger, catalogue)


def self_test() -> None:
    """Each bypass of #274 is a mutation the gate rejects."""
    trait = (
        "pub trait WalkingSurfaceService {\n"
        "    /// The flight.\n"
        "    // gate: measures length, plane_angle\n"
        "    fn measure_tread_flight(&self, r: &Request) -> Result<Flight, E>;\n"
        "    // gate: measures length\n"
        "    fn clear_width<T: Into<Id>, U>(&self, r: &HashMap<T, U>) -> Result<W, E> {\n"
        "        fn helper() {}\n"
        "        todo!()\n"
        "    }\n"
        "    // gate: reads\n"
        "    fn source_snapshots(&self) -> &[S];\n"
        "}\n"
    )
    methods, failures = service_methods(trait)
    assert not failures, failures
    assert [(m["method"], m["arity"], sorted(m["kinds"])) for m in methods] == [
        ("measure_tread_flight", 1, ["length", "plane_angle"]),
        ("clear_width", 1, ["length"]),
        ("source_snapshots", 0, []),
    ], methods
    assert methods[0]["service"] == "walking-surface"

    catalogue = {
        "measuredValues": [
            {"name": "riser", "available": True, "services": ["walking-surface"],
             "dimension": "length"},
            {"name": "tread_area", "available": True, "services": ["walking-surface"],
             "dimension": "area"},
            {"name": "step_count", "available": True, "services": ["walking-surface"],
             "dimension": None},
            {"name": "gap", "available": True, "services": ["proximity"],
             "dimension": "length"},
        ],
        "measuredMembers": [
            {"name": "steps", "available": True, "services": ["walking-surface"],
             "fields": [{"name": "open", "kind": {"type": "truth"}}]},
        ],
    }
    lib = "registry.register_measured(stair::StairMeasures)"
    files = {
        "lib.rs": lib,
        "stair.rs": (
            "impl RuleCapability for Stair {}\n"
            "fn check(s: &S) {\n    let f = s.measure_tread_flight(&r);\n"
            "    let w = s.clear_width(&r);\n    let n = f.clear_width();\n}\n"
        ),
        "stair/measured.rs": (
            "impl MeasuredProvider for StairMeasures {}\n"
            "fn f(s: &S) { s.measure_tread_flight(&r); }\n"
        ),
    }
    ledger = {
        "callSites": {
            "stair::check::measure_tread_flight": {"count": 1, "providedBy": ["riser"]},
            "stair::check::clear_width": {
                "count": 1, "searchOnly": "obstruction-search",
                "reason": "searches the flight's free zone for obstructions",
            },
        }
    }

    def gate(trait_text: str = trait, sources: dict = files, book: dict = ledger) -> list[str]:
        return violations([("core/stair.rs", trait_text)], sources, book, catalogue)

    assert gate() == [], gate()
    # A registered provider's own calls are the measured value: exempt.
    # A provider file that also holds a capability is not.
    assert gate(sources={**files, "stair/measured.rs": files["stair/measured.rs"]
                         + "impl RuleCapability for Hidden {}"})
    # An unregistered provider is no exemption.
    assert gate(sources={**files, "lib.rs": ""})

    # (b) Per call site, not per module: a second call in the same function,
    # or a call in a new function of a module that has a provider, fails.
    second = files["stair.rs"].replace("let w", "s.measure_tread_flight(&q);\n    let w")
    assert gate(sources={**files, "stair.rs": second}) == [
        "call site `stair::check::measure_tread_flight` is taken 2x but the ledger counts 1; "
        "map the new call"
    ]
    added = {**files, "stair.rs": files["stair.rs"] + "fn hide(s: &S) { s.measure_tread_flight(&r); }"}
    assert gate(sources=added) == [
        "call site `stair::hide::measure_tread_flight` takes a measuring service (1x) with no "
        "entry in scripts/measurement_ledger.json; map it to the measured value that provides "
        "it, or register one"
    ]
    # A path call, or the method passed as a function, is a call site too.
    path_call = {**files, "stair.rs": files["stair.rs"]
                 + "fn hide(s: &S) { WalkingSurfaceServiceHandle::measure_tread_flight(s, &r); }"}
    assert gate(sources=path_call)
    passed = {**files, "stair.rs": files["stair.rs"]
              + "fn hide(s: &S) { r.map(WalkingSurfaceService::measure_tread_flight); }"}
    assert gate(sources=passed)
    # The enclosing function is found past an array return type and a quote
    # in a char literal.
    arrayed = {**files, "stair.rs": files["stair.rs"]
               + "fn along(s: &S) -> [f64; 2] { let q = '\"'; s.measure_tread_flight(&r); }"}
    assert gate(sources=arrayed)[0].startswith(
        "call site `stair::along::measure_tread_flight` takes a measuring service (1x)"
    ), gate(sources=arrayed)
    # A call in a comment or string is no call; an accessor of another arity
    # (`f.clear_width()`) is not the service method.
    noise = {**files, "stair.rs": files["stair.rs"]
             + "// s.measure_tread_flight(&r)\nconst X: &str = \"s.measure_tread_flight(&r)\";"}
    assert gate(sources=noise) == []
    # A stale entry fails, so the ledger only maps call sites that exist.
    stale = {"callSites": {**ledger["callSites"],
                           "stair::gone::measure_tread_flight": {"count": 1, "providedBy": ["riser"]}}}
    assert gate(book=stale) == ["ledger entry `stair::gone::measure_tread_flight` is stale: no such call site"]

    # (a) A search reason is reviewed: a kind from the closed vocabulary and
    # a reason in words, never any non-blank string.
    def searching(kind: str, reason: str) -> dict:
        return {"callSites": {**ledger["callSites"], "stair::check::clear_width": {
            "count": 1, "searchOnly": kind, "reason": reason}}}

    assert gate(book=searching("a search", "searches the flight's free zone"))
    assert gate(book=searching("obstruction-search", " "))
    assert gate(book=searching("obstruction-search", "a search"))
    # A service call is a measurement, never an inline derivation.
    assert gate(book=searching("derivation", "converts the stated width to metres"))
    both = {"callSites": {**ledger["callSites"], "stair::check::clear_width": {
        "count": 1, "providedBy": ["riser"], "searchOnly": "obstruction-search",
        "reason": "searches the flight's free zone for obstructions"}}}
    assert gate(book=both)
    # A known unregistered measurement names the issue registering it, and
    # their number never exceeds the budget (which may be none at all).
    named = gate(book=searching("unregistered", "the width is not registered yet (#225)"))
    assert all("over the budget" in failure for failure in named), named
    unnamed = gate(book=searching("unregistered", "the width is not registered yet"))
    assert any("over the budget" not in failure for failure in unnamed), unnamed
    many = {"callSites": {**ledger["callSites"], **{
        f"stair::check::m{index}": {"count": 1, "searchOnly": "unregistered",
                                    "reason": "not registered yet (#225)"}
        for index in range(UNREGISTERED_BUDGET + 1)}}}
    assert any("over the budget" in failure for failure in gate(book=many))

    # (c) `providedBy` names a registered value measured through the called
    # service, of a kind the method declares.
    def provided(*names: str) -> dict:
        return {"callSites": {**ledger["callSites"], "stair::check::measure_tread_flight": {
            "count": 1, "providedBy": list(names)}}}

    assert gate(book=provided("unknown_value")) == [
        "ledger entry `stair::check::measure_tread_flight` names `unknown_value`, which is no "
        "registered measured value"
    ]
    assert gate(book=provided("gap")) == [
        "ledger entry `stair::check::measure_tread_flight` names `gap`, which is not measured "
        "through walking-surface"
    ]
    assert gate(book=provided("tread_area")) == [
        "ledger entry `stair::check::measure_tread_flight` names `tread_area` (area), but the "
        "call measures length, plane_angle"
    ]
    # A plain number counts or shares what the call measures.
    assert gate(book=provided("step_count")) == []
    assert gate(book=provided("steps"))  # truth members, the call measures lengths
    assert gate(book=provided())

    # (d) A measuring method not named `measure_*` is seen through its marker;
    # renaming keeps the marker; a method without one, or a measurement
    # marked `reads`, fails; inline geometry is a call site.
    renamed = trait.replace("measure_tread_flight", "tread_flight")
    hidden = {**files, "stair.rs": files["stair.rs"].replace("measure_tread_flight", "tread_flight")}
    assert gate(renamed, hidden) == [
        "call site `stair::check::tread_flight` takes a measuring service (1x) with no entry in "
        "scripts/measurement_ledger.json; map it to the measured value that provides it, or "
        "register one",
        "ledger entry `stair::check::measure_tread_flight` is stale: no such call site",
    ]
    unmarked = trait.replace("    // gate: measures length\n", "")
    assert gate(unmarked)[0] == (
        "core/stair.rs: service `WalkingSurfaceService::clear_width` has no "
        "`// gate: measures <kinds>` or `// gate: reads` marker"
    )
    assert gate(trait.replace("measures length, plane_angle", "reads"))[0] == (
        "core/stair.rs: service `WalkingSurfaceService::measure_tread_flight` is named as a "
        "measurement but marked `reads`"
    )
    assert gate(trait.replace("measures length\n", "measures furlongs\n"))
    assert gate(trait.replace("measures length\n", "measures\n"))
    inline = {**files, "stair.rs": files["stair.rs"] + "fn rise(a: f64, b: f64) -> f64 { a.hypot(b) }"}
    assert gate(sources=inline) == [
        "call site `stair::rise::hypot` takes inline geometry (1x) with no entry in "
        "scripts/measurement_ledger.json; map it to the measured value that provides it, or "
        "register one"
    ]
    derived = {"callSites": {**ledger["callSites"], "stair::rise::hypot": {
        "count": 1, "searchOnly": "derivation",
        "reason": "combines the two measured goings into the walking length"}}}
    assert gate(sources=inline, book=derived) == []
    assert gate(sources={**files, "stair.rs": files["stair.rs"] + "fn r(x: f64) { f64::sqrt(x); }"})
    # An inert gate is a failure, not a pass.
    assert violations([], files, ledger, catalogue)[0].startswith("no service method")
