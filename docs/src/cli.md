# Command line

The `axioval` binary (`axioval-cli`) is the host that composes the pieces: the
IFC source adapter reads the model, the engine runs the packages, and the BCF
sink writes issues. Each piece is also usable on its own as a library.

## `axioval validate`

```bash
axioval validate --definitions definitions.json --ruleset ruleset.json
```

Binds a ruleset to its definition packages and the built-in capabilities
without a model. Exits 0 when the ruleset compiles, 1 otherwise.

## `axioval check`

```bash
axioval check --model building.ifc \
  --definitions definitions.json --ruleset ruleset.json \
  [--geometry] [--report result.json] [--summary [--top N]] [--bcf issues.bcfzip] \
  [--bcf-author NAME] [--bcf-date 2026-09-26T10:00:00Z]
```

Runs the ruleset over an IFC2X3 or IFC4 STEP model. The source document is
named by the model's file name, so a result does not depend on the directory
it was checked from.

### Output

The JSON result goes to stdout, or to `--report`:

```json
{
  "report": { "findings": [], "not_evaluated": [] },
  "integrity": [
    { "code": "identity.invalid-global-id", "severity": "warning",
      "message": "...", "locator": "ifc:sha256:...:global-id-invalid:#2" }
  ]
}
```

`report` is the engine's `Report`. `integrity` lists irregularities of the model
itself (see [Independent adapters](./adapters.md)); they are not rule findings.
`objects` maps every object the report names to its kind and, when it has one,
its GlobalId, so a reader can tell what `#4711` is without the model:

```json
"objects": {
  "ifc-step:building.ifc/#4711": { "kind": "IFCWALL", "global_id": "2O2Fr$t4X7Zf8NOew3FLOH" }
}
```

Integrity issues and a one-line count also go to stderr.

`--summary` prints a bounded digest to stdout instead of the full JSON; see
[Reading results](#reading-results). With `--report` the full JSON is still
saved, and per-issue stderr lines are left out, because the summary already
groups them.

`--bcf` also writes a BCF 2.1 archive (see [Report sinks](./sinks.md)). The
topic date is `--bcf-date`, else `SOURCE_DATE_EPOCH` when set, else the current
time, all in UTC. With `SOURCE_DATE_EPOCH` set, the same inputs write
byte-identical archives. Objects the archive cannot select, because they have
no valid unique GlobalId, are named on stderr.

Everything is built before anything is written: a failing run leaves no
partial report or archive behind.

### Exit status

| Status | Meaning |
|---|---|
| 0 | Every rule was evaluated and nothing was found |
| 3 | At least one finding |
| 4 | No finding, but part of the check was not evaluated: the model has **not** passed |
| 1 | The check could not run: unreadable input, a package that does not compile, a model that does not import, or output the BCF writer refuses |
| 2 | Invalid command-line usage |

A finding takes precedence over incompleteness: status 3 can still come with
not-evaluated outcomes in the report. Automation that only needs pass or fail
treats any non-zero status as a failure.

## Reading results

A full result grows with the model: one entry per finding, and a real model has
thousands. `axioval report` reads a result saved by `check --report` without
re-running the check, in two views sized for a reader with a budget, such as a
person at a terminal or an LLM agent paying per token.

**Summary** (no filters, or `check --summary`): one group per rule and severity,
per rule and not-evaluated reason, and per integrity code, with its count, its
most frequent message, and up to three example objects. Its size depends on
how many distinct rules fired, not on how many objects they fired on: on a
real model with 281 findings and 95 integrity issues it is 712 bytes, against
271 KB of full JSON. Messages that differ only in the `#instance` they name
count as one message.

```text
status: findings · 281 finding(s) · 0 not evaluated · 95 integrity issue(s)

finding:
     281  error    wall-reference-required
          missing exact property axioval:example.ifc.reference
          e.g. #100410 IFCWALLSTANDARDCASE 2bGFZmGyf7awtvk0MxsCrc; …; +278 more

integrity:
      95  warning  relationship.absent-required-end
          IfcRelSpaceBoundary #157259 has no `RelatedBuildingElement`, …

next:
  axioval report r.json --rule wall-reference-required
  axioval report r.json --object '#100410' --evidence
```

**Listing** (any of `--section`, `--rule`, `--code`, `--object`): the matching
entries, `--limit` per page (default 20) from `--offset`. `--object` accepts a
local id (`#42`), a full id, or a GlobalId. Evidence locators are long and
rarely needed, so they appear only with `--evidence`.

Both views end with the exact command for the next step: the largest group,
an example object, the next page. Every suggested command is quoted for a POSIX
shell and runs unchanged. `--json` prints either view as JSON. `report` exits 0
when it could read the result and 1 otherwise; the check's own status is the
summary's `status` field.

A typical agent loop:

```bash
# status and groups, bounded
axioval check --model m.ifc --definitions d.json --ruleset r.json \
  --report result.json --summary
# one rule, 20 at a time
axioval report result.json --rule RULE
# one element in full
axioval report result.json --object '#42' --evidence
```

### Geometry

Semantic evidence (properties, relationships, classifications, the type
hierarchy) is always available. `--geometry` also meshes the model, so
geometric rules (clash, distance, contact, space and free-space checks) can
run. It is off by default because meshing costs time a purely semantic ruleset
does not need. Without it, geometric rules report `missing-service` (status 4),
never pass, and the summary suggests `--geometry`.

The CLI meshes each product's net body, with openings subtracted, using
`ifc-geometry` and hands the meshes to the Axiolid geometry services. That
bridge lives in the CLI, not in an adapter, because the IFC and Axiolid
adapters must not depend on each other. Every object ends in one of four
states:

| State | Meaning | Effect on geometric rules |
|---|---|---|
| exact | every face is planar (polygonal extrusions, faceted B-reps, meshes, booleans of these), so the mesh is the shape | measured as exact |
| tessellated | some face is curved; the mesh is within 1 mm of it | measured as approximate, never exact |
| no body | the object occupies no material: spatial structure, openings, annotations, grids, ports, structural analysis items, non-products | ignored as an obstacle |
| unmeasured | a physical product that could not be meshed, including one without a Body representation | measurements it could affect are not evaluated |

The planarity check is conservative. Anything it does not recognise counts as
tessellated, which only loses exactness, never presents an approximation as
exact.

Space validation also needs roles and storeys: `IfcSpace`, `IfcSlab`, `IfcRoof`
and `IfcBuilding` give roles, and the spatial tree gives each object's storey.
An element the file places twice, or anything under a structure aggregated
twice, gets no storey rather than a guessed one.

The result's `geometry` field records the counts and every unmeasured object
with its reason. The summary prints a `geometry:` line and groups unmeasured
objects by reason; `axioval report result.json --section geometry` lists them.

On a real 6 MiB IFC4 model, 951 bodies mesh exactly, 20 as tessellations
(round columns) and none fail, and a wall-against-wall clash check finds the
overlapping wall joints (0.05 m and 0.10 m) in under a second.

Services that need facts IFC does not state reliably are not registered, so
their rules report `missing-service`: guard checks (which surfaces are
walkable), envelope membership (which spaces bound the envelope) and shelf
capacity (doorways per room).
