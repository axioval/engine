#!/usr/bin/env python3
"""Fail closed when source/backend-specific dependencies leak into core crates."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

CORE = ("axioval-ir", "axioval-engine", "axioval-rules")
FORBIDDEN_DEPENDENCIES = ("ifc", "step", "openbim", "icdd", "axiolid", "opencascade", "cgal", "solibri")
FORBIDDEN_SOURCE = (
    re.compile(r"\b(?:use|extern\s+crate)\s+[^;]*(?:ifc|step|openbim|icdd|axiolid|opencascade|cgal|solibri)", re.I),
    re.compile(r"\b(?:ifc|step|openbim|icdd|axiolid|opencascade|cgal|solibri)(?:_[a-z0-9_]+)?\s*::", re.I),
    re.compile(r"\b(?:IfcModel|EntityRef|IfcModelSet|StepModel)\b"),
)
ACTION_USE = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)", re.M)
IMMUTABLE_ACTION = re.compile(r"^[^@]+@[0-9a-f]{40}$")


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
    assert not manifest_violations('[dependencies]\nserde = "1"\n')
    assert not source_violations("/// IFC is an adapter, not the IR.\npub struct Project;")


def check(root: Path) -> list[str]:
    failures: list[str] = []
    for crate in CORE:
        crate_root = root / "crates" / crate
        manifest = crate_root / "Cargo.toml"
        for dependency in manifest_violations(manifest.read_text(encoding="utf-8")):
            failures.append(f"{manifest.relative_to(root)}: forbidden dependency {dependency!r}")
        for source in sorted((crate_root / "src").rglob("*.rs")):
            for pattern in source_violations(source.read_text(encoding="utf-8")):
                failures.append(f"{source.relative_to(root)}: forbidden source coupling matching {pattern!r}")
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
