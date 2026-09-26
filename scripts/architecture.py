#!/usr/bin/env python3
"""Fail closed when source/backend-specific dependencies leak into core crates."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

# Crates that must stay source-neutral. Adapters, output sinks and the facade
# are exempt by name; everything else is core by default, so a newly added crate is guarded
# from its first commit rather than whenever someone remembers to list it.
ADAPTER_CRATES = frozenset(
    {
        "axioval-ifc",
        "axioval-axiolid",
        "axioval-icdd",
        "axioval-bcf",
        "axioval",
        "axioval-cli",
    }
)

# Crates live in category directories (`contracts/`, `engine/`, `sources/…`),
# so the tree depth varies and a fixed `crates/*/Cargo.toml` glob would silently
# return nothing -- a green gate that checks no crate at all. Discovery is
# recursive and keyed on the manifest's declared package name, never on its
# path, so moving a crate between categories can never disable its guard.
PACKAGE_NAME = re.compile(r"^\s*name\s*=\s*\"([^\"]+)\"", re.M)


def crate_manifests(root: Path) -> tuple[tuple[str, Path], ...]:
    """Every workspace crate as `(package name, crate root)`, found by manifest."""
    found: list[tuple[str, Path]] = []
    for manifest in sorted((root / "crates").rglob("Cargo.toml")):
        text = manifest.read_text(encoding="utf-8")
        if "[package]" not in text:
            continue
        match = PACKAGE_NAME.search(text[text.index("[package]") :])
        if match:
            found.append((match.group(1), manifest.parent))
    return tuple(found)


def core_crates(root: Path) -> tuple[tuple[str, Path], ...]:
    """Every workspace crate that is not a named adapter or frontend.

    Fails closed: an empty result means discovery broke (a moved tree, a bad
    glob), not that the workspace is adapter-only. Returning nothing would
    silently pass every neutrality check in this gate.
    """
    crates = crate_manifests(root)
    if not crates:
        raise ValueError(f"no crate manifests found under {root / 'crates'}")
    core = tuple((name, path) for name, path in crates if name not in ADAPTER_CRATES)
    if not core:
        raise ValueError(f"no core crates among {[name for name, _ in crates]}")
    return core

# Public, technology-level couplings a core crate may never take on. These are
# formats and kernels, not companies, so they belong in tracked source: a public
# clone must be able to run this gate at full strength.
#
# Private downstream party names are appended from `private/forbidden-terms.json`
# when present (see `private_couplings`). Their absence weakens this gate only
# for names that can no longer appear in the tree anyway, since naming them in
# tracked source is itself the violation.
FORBIDDEN_DEPENDENCIES = ("ifc", "step", "openbim", "icdd", "axiolid", "opencascade", "cgal")
_COUPLING_ALTERNATION = "|".join(FORBIDDEN_DEPENDENCIES)
FORBIDDEN_SOURCE = (
    re.compile(rf"\b(?:use|extern\s+crate)\s+[^;]*(?:{_COUPLING_ALTERNATION})", re.I),
    re.compile(rf"\b(?:{_COUPLING_ALTERNATION})(?:_[a-z0-9_]+)?\s*::", re.I),
    re.compile(r"\b(?:IfcModel|EntityRef|IfcModelSet|StepModel)\b"),
)
ACTION_USE = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)", re.M)
# ADR 0002: an evidence service may be parameterized by geometry and identity,
# never by a rule. A `*PlanSpec` argument, or a method named after a native rule,
# means rule policy has leaked into the evidence seam -- the exact defect that
# made the legacy 43-method `GeometryProvider` unportable one rule at a time.
# An evidence seam is any trait supplying evidence to rules: Service or
# Provider. ADR 0004 applies to both -- GeometryProvider is the trait that
# grew 17 verdict-shaped methods.
SERVICE_TRAIT = re.compile(r"pub\s+trait\s+(\w*(?:Service|Provider))\b[^{]*\{", re.M)
METHOD_NAME = re.compile(r"\bfn\s+(\w+)")
PLAN_ARGUMENT = re.compile(r"\b\w*PlanSpec\b")
PLAN_METHOD = re.compile(r"fn\s+\w+\s*\((?:[^()]|\([^()]*\))*?_?plan\s*:\s*&\w*PlanSpec\b", re.S)
PLAN_METHOD_BUDGET = 17
PLAN_ALIAS = re.compile(r"^\s*(?:pub\s+)?type\s+(\w+)\s*=\s*[^;]*\w*PlanSpec\b", re.M)
RULE_SUFFIX = re.compile(r"(?:Rule|Constraint|Check)$")
CAMEL_BOUNDARY = re.compile(r"(?<!^)(?=[A-Z])")


def rule_stems(vocabulary: str) -> set[str]:
    """Snake-case stems of every rule family the evidence seam must not name.

    Reads `scripts/rule_vocabulary.json`: generic domain rule families, carrying
    no product or consumer identity, so the gate is fully reproducible from a
    public clone. Fails loudly on schema drift -- defaulting to an empty set
    would silently disable the entire rule-name half of the gate the moment a
    key is renamed, leaving a green gate that checks nothing.
    """
    document = json.loads(vocabulary)
    if not isinstance(document, dict) or not isinstance(document.get("stems"), list):
        raise ValueError("rule vocabulary must be an object with a `stems` list")
    stems = {
        CAMEL_BOUNDARY.sub("_", RULE_SUFFIX.sub("", stem)).lower()
        for stem in document["stems"]
        if isinstance(stem, str) and stem
    }
    if not stems:
        raise ValueError("rule vocabulary names no rules; the seam gate would be inert")
    return stems


def strip_noise(source: str) -> str:
    """Blank out comments and string literals, preserving byte offsets.

    The brace matcher below counts `{`/`}` to find a trait body. A brace inside
    a doc comment or string would close the body early and hide every violation
    after it, so those regions are neutralised first.
    """
    out = list(source)
    index, size = 0, len(source)
    while index < size:
        pair = source[index : index + 2]
        if pair in ("//", "/*"):
            terminator, skip = ("\n", 1) if pair == "//" else ("*/", 2)
            stop = source.find(terminator, index + 2)
            stop = size if stop < 0 else stop + skip
            for position in range(index, stop):
                if out[position] != "\n":
                    out[position] = " "
            index = stop
            continue
        if source[index] == '"':
            index += 1
            while index < size and source[index] != '"':
                index += 2 if source[index] == "\\" else 1
                out[index - 1] = " "
            index += 1
            continue
        index += 1
    return "".join(out)


def trait_bodies(source: str) -> list[tuple[str, str]]:
    """(trait name, body) for each public service trait, brace-matched."""
    scan = strip_noise(source)
    bodies: list[tuple[str, str]] = []
    for match in SERVICE_TRAIT.finditer(scan):
        depth, index = 1, match.end()
        while index < len(scan) and depth:
            depth += {"{": 1, "}": -1}.get(scan[index], 0)
            index += 1
        bodies.append((match.group(1), scan[match.end() : index - 1]))
    return bodies


def method_signatures(body: str) -> list[tuple[str, str, str]]:
    """(name, signature, return type) per method, tolerating generics."""
    signatures: list[tuple[str, str, str]] = []
    for match in METHOD_NAME.finditer(body):
        open_paren = body.find("(", match.end())
        if open_paren < 0:
            continue
        depth, index = 1, open_paren + 1
        while index < len(body) and depth:
            depth += {"(": 1, ")": -1}.get(body[index], 0)
            index += 1
        # Return type: everything up to the method terminator, so a
        # verdict-shaped result is visible to ADR 0004.
        tail = body[index:]
        stop = min((x for x in (tail.find(chr(59)), tail.find(chr(123))) if x >= 0), default=len(tail))
        signatures.append((match.group(1), body[open_paren:index], tail[:stop]))
    return signatures


# ADR 0004: a service returns what was MEASURED; a capability decides what it
# means. A return type naming a finding/violation/compliance verdict means rule
# policy was computed behind the evidence seam -- the defect that made 17 of 23
# plan-shaped provider methods unportable one rule at a time.
VERDICT_RETURN = re.compile(r"\b\w*(?:Finding|Violation|Compliance|Verdict)\w*\b")


def verdict_return_violations(source: str) -> list[str]:
    """Service methods returning a decided verdict rather than a measurement."""
    failures: list[str] = []
    for trait, body in trait_bodies(source):
        for method, _signature, ret in method_signatures(body):
            for verdict in sorted(set(VERDICT_RETURN.findall(ret))):
                failures.append(
                    f"service `{trait}::{method}` returns verdict `{verdict}`"
                )
    return failures


def service_seam_violations(source: str, stems: set[str]) -> list[str]:
    """Reject rule policy embedded in an evidence-service interface."""
    aliases = set(PLAN_ALIAS.findall(source))
    plan_types = PLAN_ARGUMENT
    failures: list[str] = []
    for trait, body in trait_bodies(source):
        for method, signature, _ret in method_signatures(body):
            plans = set(plan_types.findall(signature))
            plans |= {alias for alias in aliases if re.search(rf"\b{alias}\b", signature)}
            for plan in sorted(plans):
                failures.append(f"service `{trait}::{method}` takes rule plan `{plan}`")
            # Token containment, not prefix-strip: `get_stair`, `stair_lookup`
            # and `resolve_stair` are the same leak wearing different verbs.
            tokens = set(method.split("_"))
            for stem in sorted(stems):
                parts = stem.split("_")
                if set(parts) <= tokens and "_".join(parts) in method:
                    failures.append(
                        f"service `{trait}::{method}` is named after rule `{stem}`"
                    )
                    break
    return failures


# ADR 0003: an adapter must mint `ObjectId` from a source-stable identifier.
# A per-model arena index or parse position is not durable identity: it does not
# survive reparse or a differing importer, so findings keyed by it cannot be
# compared across runs. Minting from an index-typed local id is rejected.
ARENA_IDENTITY = re.compile(
    r"ObjectId::new\s*\(.*?,\s*(?:[\w.]*\.)?\b(?:"
    r"(?:entity|element|node|item|slot)(?:_?(?:ref|idx|index|pos|position|offset))?"
    r")\b\s*(?:\.to_string\(\)|\.0\b|as\s+u(?:32|64)|\.index\(\))",
    re.S | re.I,
)


def arena_identity_violations(source: str) -> list[str]:
    """Reject ObjectId minted from an in-memory index rather than a stable id."""
    code = strip_noise(source)
    return [
        match.group(0).split("(")[0].strip()
        for match in ARENA_IDENTITY.finditer(code)
    ]


IMMUTABLE_ACTION = re.compile(r"^[^@]+@[0-9a-f]{40}$")
DEPENDENCY_AUDIT_MARKER = "AXIOVAL_DEPENDENCY_AUDIT_COMPLETE"
CARGO_DENY_ACTION = re.compile(
    r"uses:\s*EmbarkStudios/cargo-deny-action@[0-9a-f]{40}", re.I
)


def dependency_names(manifest: str) -> set[str]:
    data = tomllib.loads(manifest)
    names: set[str] = set()
    sections = ("dependencies", "dev-dependencies", "build-dependencies")
    for section in sections:
        for name, declaration in data.get(section, {}).items():
            names.add(name)
            if isinstance(declaration, dict) and "package" in declaration:
                names.add(declaration["package"])
    for target in data.get("target", {}).values():
        for section in sections:
            for name, declaration in target.get(section, {}).items():
                names.add(name)
                if isinstance(declaration, dict) and "package" in declaration:
                    names.add(declaration["package"])
    return names


def manifest_violations(manifest: str) -> list[str]:
    return sorted(
        name
        for name in dependency_names(manifest)
        if any(token in name.casefold() for token in FORBIDDEN_DEPENDENCIES)
    )


def source_violations(source: str) -> list[str]:
    code = "\n".join(line for line in source.splitlines() if not line.lstrip().startswith("//"))
    return [pattern.pattern for pattern in FORBIDDEN_SOURCE if pattern.search(code)]


def workflow_violations(source: str) -> list[str]:
    failures: list[str] = []
    for action in ACTION_USE.findall(source):
        if not action.startswith("./") and not IMMUTABLE_ACTION.fullmatch(action):
            failures.append(f"mutable action reference {action!r}")
    if "curl " in source and "sha256sum --check --strict" not in source:
        failures.append("download executes without SHA-256 verification")
    if DEPENDENCY_AUDIT_MARKER in source:
        marker_offset = source.index(DEPENDENCY_AUDIT_MARKER)
        if not CARGO_DENY_ACTION.search(source[:marker_offset]):
            failures.append("dependency-audit marker lacks a preceding pinned cargo-deny action")
    return failures


def self_test() -> None:
    assert manifest_violations('[dependencies]\nopenbim-ifc = "1"\n') == ["openbim-ifc"]
    assert manifest_violations(
        '[dependencies]\nmodel = { package = "openbim-ifc", version = "1" }\n'
    ) == ["openbim-ifc"]
    assert source_violations("use openbim_ifc::Model;")
    assert source_violations("fn leak(model: ifc::Model) {}")
    assert source_violations("fn leak(model: step::Model) {}")
    assert source_violations("fn leak(_: IfcModel<'_>) {}")
    assert workflow_violations("- uses: actions/checkout@v4")
    assert not workflow_violations(
        "- uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262"
    )
    assert workflow_violations("run: curl https://example.invalid/tool | sh")
    assert workflow_violations(
        "env:\n  AXIOVAL_DEPENDENCY_AUDIT_COMPLETE: '1'\nrun: ./scripts/check.sh"
    )
    assert not workflow_violations(
        "- uses: EmbarkStudios/cargo-deny-action@"
        "3c6349835b2b7b196a839186cb8b78e02f7b5f25\n"
        "- env:\n    AXIOVAL_DEPENDENCY_AUDIT_COMPLETE: '1'\n"
    )
    assert not manifest_violations('[dependencies]\nserde = "1"\n')
    assert not source_violations("/// IFC is an adapter, not the IR.\npub struct Project;")
    vocabulary = '{"stems": ["StairRule", "free_floor_space"]}'
    stems = rule_stems(vocabulary)
    assert stems == {"stair", "free_floor_space"}
    assert service_seam_violations(
        "pub trait StairService {\n    fn resolve_stair(&self, plan: &StairPlanSpec) -> u8;\n}", stems
    ) == [
        "service `StairService::resolve_stair` takes rule plan `StairPlanSpec`",
        "service `StairService::resolve_stair` is named after rule `stair`",
    ]
    assert service_seam_violations(
        "pub trait FreeSpaceService {\n    fn free_floor_space(&self, o: ObjectId) -> u8;\n}", stems
    ) == ["service `FreeSpaceService::free_floor_space` is named after rule `free_floor_space`"]
    # Neutral evidence: parameterized by identity and geometry, not by a rule.
    assert not service_seam_violations(
        "pub trait FreeSpaceService {\n    fn largest_inscribed_circle(&self, o: ObjectId) -> u8;\n}",
        stems,
    )
    # ADR 0003: identity must come from a stable source id, not an arena index.
    assert arena_identity_violations(
        'ObjectId::new(source.clone(), entity.to_string())'
    ) == ["ObjectId::new"]
    # The real legacy sidecar shape: multi-line, field-qualified arena index.
    assert arena_identity_violations(
        "ObjectId::new(\n"
        "    SourceId::new(IFC_STEP_SYSTEM, entity.model_id.clone()).ok()?,\n"
        "    entity.entity.to_string(),\n"
        ")"
    ) == ["ObjectId::new"]
    assert arena_identity_violations("ObjectId::new(src, element_index.to_string())")
    assert arena_identity_violations("ObjectId::new(src, node_ref.0)")
    assert arena_identity_violations("ObjectId::new(src, slot as u32)")
    assert arena_identity_violations("ObjectId::new(src, item.index())")
    # A STEP entity id or GlobalId is a stable source identifier.
    assert not arena_identity_violations("ObjectId::new(source.clone(), id.to_string())")
    assert not arena_identity_violations("ObjectId::new(src, global_id.to_string())")
    assert not arena_identity_violations("// ObjectId::new(src, entity.to_string())")

    # A capability may name a rule -- policy belongs in `axioval-rules`.
    assert not service_seam_violations(
        "pub trait RuleCapability {\n    fn resolve_stair(&self, plan: &StairPlanSpec) -> u8;\n}", stems
    )

    # Core membership is derived, not listed: a new crate is guarded on arrival.
    root = Path(__file__).resolve().parents[1]
    derived = {name for name, _ in core_crates(root)}
    assert "axioval-spec" in derived, derived
    assert "axioval-engine" in derived, derived
    assert "axioval-ifc" not in derived, derived
    assert "axioval-cli" not in derived, derived

    # Discovery is recursive and name-keyed: crates nested in category
    # directories are still found, and a core crate cannot escape the guard by
    # moving. A flat-glob implementation would return nothing here.
    discovered = {name for name, _ in crate_manifests(root)}
    assert "axioval-ifc" in discovered, discovered
    assert "axioval-engine" in discovered, discovered
    for name, path in crate_manifests(root):
        assert (path / "Cargo.toml").is_file(), (name, path)


    # --- regressions for reviewed bypasses (deleg_a59d2236, task 2) ---

    # 1. Unlisted verb prefix must not launder a rule-named method.
    for name in ("get_stair", "fetch_stair", "stair_lookup"):
        assert service_seam_violations(
            f"pub trait S Service {{\n    fn {name}(&self, o: ObjectId) -> u8;\n}}".replace(
                "S Service", "StairService"
            ),
            stems,
        ), name

    # 2. A brace inside a doc comment must not close the trait body early.
    assert service_seam_violations(
        "pub trait StairService {\n"
        "    /// e.g. HashMap::from([(\"k\", 1)]) }\n"
        "    fn resolve_stair(&self, plan: &StairPlanSpec) -> u8;\n}",
        stems,
    )

    # 3. A type alias must not hide the plan argument.
    assert service_seam_violations(
        "type StairArgs = StairPlanSpec;\n"
        "pub trait StairService {\n    fn evidence(&self, plan: &StairArgs) -> u8;\n}",
        stems,
    ) == ["service `StairService::evidence` takes rule plan `StairArgs`"]

    # 4. Generic parameters must not skip the method entirely.
    assert service_seam_violations(
        "pub trait StairService {\n"
        "    fn resolve_stair<T: Send>(&self, plan: &StairPlanSpec) -> T;\n}",
        stems,
    )

    # 5. Vocabulary schema drift must fail loudly, never silently disarm the gate.
    for broken in ('{"rules": []}', '{"stems": {}}', '{"stems": []}', '{"stems": [""]}', "[]"):
        try:
            rule_stems(broken)
        except ValueError:
            continue
        raise AssertionError(f"vocabulary drift accepted: {broken}")

    # The shipped vocabulary must itself bind: an empty or renamed file would
    # leave the seam gate inert while still reporting success.
    _shipped = rule_stems(
        (Path(__file__).resolve().parent / "rule_vocabulary.json").read_text(encoding="utf-8")
    )
    assert len(_shipped) >= 60, f"rule vocabulary shrank to {len(_shipped)} stems"

    # Neutral evidence with an incidental substring stays clean.
    assert not service_seam_violations(
        "pub trait FreeSpaceService {\n    fn staircase_free_area(&self, o: ObjectId) -> u8;\n}",
        {"stair"},
    )


    # ADR 0004: the real GeometryProvider shape. The provider decided the
    # verdict, so the rule layer had nothing portable to stand on.
    assert verdict_return_violations(
        "pub trait GeometryProvider {\n    fn resolve_stair(&self, plan: &StairPlanSpec) -> QueryOutcome<ResolvedStairFinding>;\n}"
    ) == [
        "service `GeometryProvider::resolve_stair` returns verdict `ResolvedStairFinding`",
    ]
    # A service that returns a measurement is exactly what ADR 0004 wants.
    assert not verdict_return_violations(
        "pub trait FreeSpaceService {\n    fn measure_free_area(&self, r: &FreeAreaRequest) -> Result<FreeAreaEvidence, E>;\n}"
    )
    # ADR 0004 ratchet: an unavailable source is not a violation (CI), but
    # a source over budget is. Proven against the real 23-method provider.
    assert not source_ratchet(None, 23)
    assert not source_ratchet(Path("/nonexistent/extraction/source"), 23)
    _real_source = extraction_source()
    if _real_source and (_real_source / "crates/rules/src/evidence/core/source.rs").is_file():
        assert not source_ratchet(_real_source, PLAN_METHOD_BUDGET)
        assert source_ratchet(_real_source, PLAN_METHOD_BUDGET - 1)



def private_terms() -> tuple[str, ...]:
    """Private party names that must never appear in tracked text, casefolded.

    Loaded from `private/forbidden-terms.json` (untracked) or the path in
    `AXIOVAL_FORBIDDEN_TERMS`. Returns empty when unavailable: a public clone
    genuinely cannot check for names it is not allowed to know, and failing the
    build there would make the repository unusable by contributors. The
    enforcing run is the maintainer's, where the file is present.
    """
    configured = os.environ.get("AXIOVAL_FORBIDDEN_TERMS", "").strip()
    path = Path(configured) if configured else Path(__file__).resolve().parents[1] / "private" / "forbidden-terms.json"
    if not path.is_file():
        return ()
    terms = json.loads(path.read_text(encoding="utf-8")).get("terms")
    if not isinstance(terms, list) or not terms:
        raise ValueError(f"{path}: `terms` must be a non-empty list")
    return tuple(term.casefold() for term in terms if isinstance(term, str) and term)


def tracked_files(root: Path) -> list[str]:
    """Files git actually tracks -- the exact set that reaches the public remote.

    Scanning the working tree instead would both miss nothing and check far too
    much (ignored `private/`, `target/`), so the tracked set is the only honest
    definition of "what we publish".
    """
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        capture_output=True,
        text=True,
        check=True,
    )
    return [name for name in result.stdout.split("\0") if name]


def disclosure_violations(root: Path) -> list[str]:
    """Tracked files naming a private downstream party.

    The repository is public. A vendor name reaching `main` is disclosed the
    moment it is pushed and survives in history even after deletion, so this is
    checked on every run rather than at release time.
    """
    terms = private_terms()
    if not terms:
        return []
    failures: list[str] = []
    for name in tracked_files(root):
        path = root / name
        if not path.is_file() or path.is_symlink():
            continue
        try:
            body = path.read_text(encoding="utf-8").casefold()
        except (UnicodeDecodeError, OSError):
            continue
        if any(term in body for term in terms):
            failures.append(f"{name}: names a private downstream party")
    return failures


def extraction_source() -> Path | None:
    """Private downstream checkout the ADR 0004 ratchet guards, or `None`.

    Supplied via `AXIOVAL_EXTRACTION_SOURCE` so the path to a private consumer
    is never written into this public repository. Absent means "not available
    here" (the CI case), which is not a violation; the ratchet below is a
    regression guard on a source only maintainers can see.
    """
    configured = os.environ.get("AXIOVAL_EXTRACTION_SOURCE", "").strip()
    return Path(configured) if configured else None


def source_ratchet(source_root: Path | None, budget: int) -> list[str]:
    # Guard the extraction SOURCE against regression. ADR 0004 gates the
    # destination, but nothing stopped the downstream source growing a 24th
    # plan-shaped provider method. This is a one-way ratchet: the count
    # may fall as methods are decomposed, never rise.
    if source_root is None:
        return []
    provider = source_root / "crates/rules/src/evidence/core/source.rs"
    if not provider.is_file():
        return []
    found = len(PLAN_METHOD.findall(provider.read_text(encoding="utf-8")))
    if found > budget:
        return [
            f"plan-shaped provider methods rose to {found} (budget {budget}); "
            "ADR 0004 decomposes these into measurement plus policy"
        ]
    return []




def check(root: Path) -> list[str]:
    failures: list[str] = []
    stems = rule_stems((root / "scripts" / "rule_vocabulary.json").read_text(encoding="utf-8"))
    for crate, crate_root in core_crates(root):
        manifest = crate_root / "Cargo.toml"
        for dependency in manifest_violations(manifest.read_text(encoding="utf-8")):
            failures.append(f"{manifest.relative_to(root)}: forbidden dependency {dependency!r}")
        for source in sorted((crate_root / "src").rglob("*.rs")):
            text = source.read_text(encoding="utf-8")
            for pattern in source_violations(text):
                failures.append(f"{source.relative_to(root)}: forbidden source coupling matching {pattern!r}")
            for detail in service_seam_violations(text, stems):
                failures.append(f"{source.relative_to(root)}: {detail}")
            for detail in verdict_return_violations(text):
                failures.append(f"{source.relative_to(root)}: {detail}")
    # The repository is public: a private party named in a tracked file is
    # disclosed the moment it is pushed, and deleting it later does not undo
    # the disclosure. Checked every run, not at release time.
    failures.extend(disclosure_violations(root))
    # ADR 0004 ratchet on the extraction SOURCE. The source is a private
    # downstream checkout, so its location is supplied by the environment and
    # is never named in this repository. Optional by design: the gate runs in
    # CI where the source is absent, and a missing source is not a violation --
    # but when present it must never grow a new plan-shaped provider method.
    # Budget falls as methods are decomposed.
    failures.extend(source_ratchet(extraction_source(), PLAN_METHOD_BUDGET))
    # ADR 0003 applies to every crate: adapters are exactly where identity is
    # minted, so exempting them would exempt the only code that can violate it.
    for source in sorted((root / "crates").rglob("src/**/*.rs")):
        for detail in arena_identity_violations(source.read_text(encoding="utf-8")):
            failures.append(
                f"{source.relative_to(root)}: ObjectId minted from arena index `{detail}`"
            )
    for workflow in sorted((root / ".github" / "workflows").glob("*.yml")):
        for detail in workflow_violations(workflow.read_text(encoding="utf-8")):
            failures.append(f"{workflow.relative_to(root)}: {detail}")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    root = Path(__file__).resolve().parents[1]
    failures = check(root)
    if failures:
        print("architecture boundary violations:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("architecture boundaries: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
