# Command line

The `axioval` binary (`axioval-cli`) is the host that composes the pieces: the
IFC source adapter reads the model, the engine runs the packages, and the BCF
sink writes issues. Each piece is also usable on its own as a library.

## `axioval validate`

```bash
axioval validate --definitions definitions.json --ruleset ruleset.json [--ruleset other.json ...]
```

Binds a ruleset to its definition packages and the built-in capabilities
without a model. Exits 0 when the ruleset compiles, 1 otherwise. Several
`--ruleset`s are bound together as `check` binds them.

## `axioval check`

```bash
axioval check --model building.ifc[:DISCIPLINE] [--model other.ifc[:DISCIPLINE] ...] \
  [--discipline-map FIELD:PATTERN=DISCIPLINE ...] \
  --definitions definitions.json --ruleset ruleset.json [--ruleset other.json ...] \
  [--geometry] [--locate storeys|containers|geometry] [--rule-status] [--report result.json] \
  [--summary [--top N]] [--bcf issues.bcfzip] \
  [--bcf-author NAME] [--bcf-date 2026-09-26T10:00:00Z] [--bcf-version 2.1|3.0] \
  [--bcf-subject-color HEX] [--bcf-related-color HEX] [--bcf-no-color] \
  [--bcf-isolate] [--bcf-section-box]
```

Runs the ruleset over one or more IFC2X3 or IFC4 STEP models. Each model is
one source, named by its file name, so a result does not depend on the
directory it was checked from.

### Several rulesets

A check commonly bundles several rulesets, one per discipline or per client
requirement set. `--ruleset` may be repeated: each ruleset is compiled on its
own, against the definition packages it declares, exactly as it would be
alone, and all their rules run in one check. Each rule id is then qualified
by its ruleset's package id, `package-id/rule-id`, so two rulesets that both
define `r1` report `org.example.client/r1` and `org.example.discipline/r1`,
and findings, not-evaluated outcomes, tables, the summary and `report --rule`
group by package in rule id order. With one `--ruleset` the ids stay as
written. Two rulesets with one package id, or definition packages declaring
one concept twice across the rulesets, are refused (status 1). The library
entry point is `compile_rulesets`.

### Several models and disciplines

`--model` may be repeated. Every model is imported as its own source and all
of them are checked in one session, so a rule sees every object of every
model under its source-qualified identity (`ifc-step:arch.ifc/#42` and
`ifc-step:struct.ifc/#42` are two objects). Two models with the same file name
would be one source and are refused (status 1); rename one.

A model may declare the discipline it plays with `:DISCIPLINE` after the path:

```bash
axioval check --model arch.ifc:architecture --model struct.ifc:structure \
  --definitions d.json --ruleset clash-matrix.json --geometry
```

A discipline is a lowercase token: letters `a`–`z`, digits, `-` and `_`,
starting with a letter or digit, at most 64 characters. The text after the
last `:` is the discipline when it is such a name, so a Windows drive
(`C:\models\arch.ifc`) or a directory containing `:` stays part of the path.
Text after the last `:` that looks like a name but is not a valid one
(`arch.ifc:Architecture`) is a usage error (status 2), never read as part of
the file name. A trailing `:` declares no discipline, for a file whose name
itself ends in `:name` (`--model 'odd:name:'`).

Rules scope themselves to disciplines with the `discipline` selector (see
[Discipline selectors](./capabilities.md#discipline-selectors)). A model
without a discipline has none: a discipline-scoped rule reports its objects
not evaluated, once for the model, and the check exits 4 rather than pass.

With `--geometry`, every model is meshed into one geometry set, so a clash
between an object of one file and an object of another is an ordinary pair.
The models must share one coordinate system, as models exchanged for
coordination do; the CLI does not move one onto another.

Each model's file name is stated as its source's `fileName` metadata, beside the application and project the IFC adapter reads, for `source` selectors (see
[Source selectors](./capabilities.md#source-selectors)).

Reports, `report` and BCF work with several models. Over several documents,
the summary and listings name objects as `arch.ifc/#42`, and `--object`
accepts that form.

### Disciplines from what a model states

Federated models often carry their discipline only in the application that
wrote them or in their file name. `--discipline-map FIELD:PATTERN=DISCIPLINE`
assigns one to each model that declares none:

```bash
axioval check --model a.ifc --model s.ifc --model m.ifc:mep \
  --discipline-map 'application:*Architecture*=architecture' \
  --discipline-map 'fileName:*_STR*=structure' \
  --definitions d.json --ruleset r.json
```

FIELD is `application`, `fileName`, `project` or `schema` (see
[Source selectors](./capabilities.md#source-selectors)); PATTERN is a
wildcard pattern over the whole value, as the `like` operator reads it (`*`,
`?`, `\` escapes), compared case-sensitively; the discipline is the text
after the last `=`. Rules are tried in order, and the first whose pattern
matches one of the model's values assigns its discipline. A declared
`:DISCIPLINE` always wins. A model no rule matches keeps none and behaves as
without a map. So does a model that never stated the field a rule reads
before any rule matched, since that rule might have matched. A malformed rule
is a usage error (status 2).

The result's `sources` lists every model with its discipline and where it
came from, so an assignment can be reviewed:

```json
"sources": [
  { "source": "ifc-step:a.ifc", "discipline": "architecture", "discipline_origin": "mapped",
    "mapped_by": "application:*Architecture*=architecture",
    "mapped_value": "Modeller Architecture 2024" },
  { "source": "ifc-step:m.ifc", "discipline": "mep", "discipline_origin": "declared" },
  { "source": "ifc-step:x.ifc", "unmapped": "no rule of the discipline map matches" }
]
```

A selection resting on a mapped discipline also cites the rule and the value
as inexact evidence
(`discipline-map:application:*Architecture*=architecture@Modeller Architecture 2024`)
wherever the capability keeps its selection's evidence.

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

`report` is the engine's `Report`; when rules measured values it also has
`tables` (see [Tables](./ir.md#tables)). `integrity` lists irregularities of the model
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

`--bcf` also writes a BCF archive (see [Report sinks](./sinks.md)). The
topic date is `--bcf-date`, else `SOURCE_DATE_EPOCH` when set, else the current
time, all in UTC. With `SOURCE_DATE_EPOCH` set, the same inputs write
byte-identical archives. Objects the archive cannot select, because they have
no valid unique GlobalId, are named on stderr. Each topic is labelled with its
rule id, then the rule's folder path (`Folder: Structure / Walls`) and tags
from the ruleset.

With `--geometry`, each viewpoint gets a perspective and an orthogonal camera
fitted to the measured bounds of the topic's objects. An object that was not
measured leaves its viewpoint without a camera and is named on stderr.
Without `--geometry` the archive is the same as before cameras existed.

With `--geometry`, each viewpoint also colours the finding's subject red
and its related objects blue. `--bcf-subject-color` and
`--bcf-related-color` take other colours as `RRGGBB` or `AARRGGBB` hex
digits, and colour viewpoints without `--geometry` too; `--bcf-no-color`
writes no colouring at all.

`--bcf-isolate` shows only the involved objects in each viewpoint: the rest
of the model is hidden (`DefaultVisibility` false, with the subject and
related objects as exceptions). Without it the whole model stays visible.

`--bcf-section-box` cuts each viewpoint with a camera by six clipping planes
boxing the topic's objects half a metre beyond their measured bounds. It
needs `--geometry`: a viewpoint without a camera has no bounds and is never
clipped.

`--bcf-version` is `2.1` (default) or `3.0`. BCF 3.0 requires a camera on
every viewpoint, so it needs `--geometry` and bounds for every selected
object; otherwise the run fails with status 1 and nothing is written.

`--locate` locates every finding and not-evaluated outcome by storey and
space (see [Locations](./refinement.md#locations)): `storeys` climbs
`IfcRelContainedInSpatialStructure` and `IfcRelAggregates` to
`IfcBuildingStorey`s, `containers` also to `IfcSpace`s, and `geometry` takes
spaces from the bodies that contain or meet each object, so it needs
`--geometry`. Places are named by their `Name`. Each outcome then has a
`location`, BCF topics are labelled `Storey: <name>` and `Space: <name>`,
and listings print it. Without `--locate` (or with `--locate none`) the
result is unchanged, byte for byte.

`--rule-status` adds each rule's counts and status to the report (see
[Rule status](./refinement.md#rule-status)), and the summary lists them, the
rules that did not pass first, at most `--top`:

```text
rules: 1 failed · 1 nothing selected
  failed                10 checked · 2 failed · 0 not evaluated  wall-reference-required
  nothing selected       0 checked · 0 failed · 0 not evaluated  column-reference-required
```

Without it the result is unchanged. A rule that selected nothing does not
change the exit status.

Every finding has an `id`, its stable identity over GlobalIds (see
[Review decisions](./decisions.md#finding-identity)), equal to its BCF topic
GUID. `--decisions FILE` carries the decisions in `FILE` (written by
[`axioval decide`](#axioval-decide)) over to the findings with the same
identity: each gets a `decision`, flagged `changed` when its severity or
evidence counts differ from when it was decided, and decisions whose finding
is gone are listed in the report's `stale_decisions`. The BCF archive then
writes accepted and rejected topics with that status and the decision as a
comment. Decisions never hide a finding or a not-evaluated outcome and never
change the exit status.

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

## `axioval decide`

Records a reviewer's decision about findings of a result saved with `check
--report`, in a decisions file that `check --decisions` reads:

```bash
axioval check --model rev1.ifc ... --report r1.json
axioval report r1.json --rule wall-reference-required   # lists each finding's id
axioval decide r1.json --decisions decisions.json \
  --finding 5c1f0c9e-6a0b-5d53-9a8e-2f3b8f6c1d20 --status accepted \
  --author "A. Reviewer" --comment "agreed with the architect"
axioval check --model rev2.ifc ... --decisions decisions.json --report r2.json --bcf r2.bcfzip
```

`--finding` repeats to decide several findings alike; `--status` is
`accepted`, `rejected` or `open`. The file is created when missing, and a
decision about a finding already decided replaces the earlier one. Each
decision records the finding's basis (rule, message, severity, evidence
counts), so a re-check can tell whether it changed. `--date` is an ISO 8601
date-time with offset, else `SOURCE_DATE_EPOCH` when set, else now, in UTC.
A finding the result does not contain fails the command (status 1) and
nothing is written.

## `axioval compare`

```bash
axioval compare --base r1/model.ifc --revised r2/model.ifc \
  [--property SET.NAME ...] [--property-set SET ...] [--all-property-sets] \
  [--geometry] [--timestamps] \
  [--length-tolerance METRES] [--angle-tolerance DEGREES] \
  [--report result.json] [--summary [--top N]] [--bcf changes.bcfzip] \
  [--bcf-author NAME] [--bcf-date 2026-09-26T10:00:00Z] [--bcf-version 2.1|3.0] \
  [--bcf-subject-color HEX] [--bcf-related-color HEX] [--bcf-no-color] \
  [--bcf-isolate] [--bcf-section-box]
```

Compares two revisions of one IFC2X3 or IFC4 model object by object (see
[Model comparison](./comparison.md)). Objects are matched by `GlobalId`, so a
re-export that renumbers every entity still matches. Each revision is its own
source, named by its file name; when both files have the same name, as two
revisions usually do, the sources are named `model.ifc@base` and
`model.ifc@revised`.

Compared facets:

- kind, classifications and relationships, always;
- each `--property`, as `SET.NAME` (split at the first `.`, so
  `Pset_WallCommon.FireRating` or `axioval:attributes.Name`) or a bare `NAME`;
- every property of each `--property-set`, or with `--all-property-sets` of
  every set, listed on both sides through property enumeration;
- placement (origin distance and axis rotation of each object frame), always;
- the coordinate system of the two files (world frame, true north, map
  conversion), always;
- with `--timestamps`, the header timestamps: a revised file written before
  the base is an error finding;
- with `--geometry`, both revisions are meshed as for `check --geometry` and
  each object's measured bounds are compared.

A length differs when it exceeds `--length-tolerance` (default 0.005 m) and an
angle when it exceeds `--angle-tolerance` (default 0.01°). The length default
is above twice the 1 mm chord deviation of a tessellated body, so an unchanged
curved body is provably unchanged. A tessellated difference straddling the
tolerance is undetermined, never rounded to changed or unchanged. A negative
or non-finite tolerance is refused (status 1).

### Output

The result has the shape of a `check` result: `report`, `integrity` (of both
files), `objects` (both revisions' objects, by full id) and, with
`--geometry`, `geometry` (counts over both files). The report holds one rule
id per kind of entry: `compare.added`, `compare.removed`, one finding per
changed facet (`compare.property`, `compare.placement`, `compare.geometry`,
...) on the revised object with its base object in `related`,
`compare.coordinate-system` on the revised source, and not-evaluated outcomes
for facets that could not be compared, undetermined measures and unmatched
objects (`compare.identity`). So `--summary`, `axioval report` and `--bcf`
work on a comparison exactly as on a check, and `report --rule
compare.placement` lists the moved objects.

The result also has a `comparison` field with the structured comparison:

```json
"comparison": {
  "base": "ifc-step:model.ifc@base",
  "revised": "ifc-step:model.ifc@revised",
  "scheme": "ifc-globalid",
  "facets": ["kind", "classifications", "property", "relationship",
             "placement", "geometry", "coordinate-system"],
  "tolerance": { "length_metres": 0.005, "angle_degrees": 0.01 },
  "counts": { "added": 1, "removed": 1, "changed": 2, "unchanged": 1,
              "incomplete": 0, "unidentified": 0, "ambiguous": 0 },
  "objects": [
    { "identity": "0000000000000000000A01", "state": "changed", "kind": "IFCWALL",
      "base": { "source": { "system": "ifc-step", "document": "model.ifc@base" }, "local_id": "#109" },
      "revised": { "source": { "system": "ifc-step", "document": "model.ifc@revised" }, "local_id": "#549" },
      "changes": [
        { "facet": "placement", "measure": "origin", "unit": "m",
          "lower": 0.5, "upper": 0.5, "tolerance": 0.005,
          "detail": "placement origin differs by 0.5000 m (tolerance 0.0050 m)" }
      ] }
  ],
  "unidentified": [],
  "ambiguous": [],
  "coordinate_systems": [
    { "base": { "system": "ifc-step", "document": "model.ifc@base" },
      "revised": { "system": "ifc-step", "document": "model.ifc@revised" } }
  ]
}
```

`objects` lists every identity that is `added`, `removed`, `changed` or
`incomplete` (matched with no difference but a facet not compared or a
measure undetermined), in identity order; unchanged identities are only
counted. Each change carries its `facet` and a `detail`; a measured change
also its `measure`, the interval `lower`–`upper` it lies in, the `tolerance`
and the `unit` (`m`, `rad`, or empty for the map scale). `unresolved` and
`undetermined` list what was not decided. A result without a `comparison`
field is a check's; the field is additive.

`--summary` starts with what was compared and the counts:

```text
compared: ifc-step:model.ifc@base -> ifc-step:model.ifc@revised · kind, classifications, property, relationship, placement, geometry, coordinate-system
objects: 1 added · 1 removed · 2 changed · 1 unchanged · 0 incomplete · 0 unidentified · 0 ambiguous
status: findings · 5 finding(s) · 0 not evaluated · 0 integrity issue(s)
```

followed by the same groups and next steps as a check's summary.

### Exit status

As for `check`: 0 when the revisions agree on everything compared, 3 when
anything was added, removed or changed, 4 when nothing differs but something
could not be compared (a facet not compared, an undetermined measure, an
unmatched object), 1 when the comparison could not run, 2 for invalid usage.
Everything is built before anything is written.

## Reading results

A full result grows with the model: one entry per finding, and a real model has
thousands. `axioval report` reads a result saved by `check --report` or
`compare --report` without re-running it, in two views sized for a reader with a budget, such as a
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

**Listing** (any of `--section`, `--rule`, `--code`, `--object`,
`--location`): the matching entries, `--limit` per page (default 20) from
`--offset`. `--object` accepts a local id (`#42`), a full id, or a GlobalId.
Evidence locators are long and rarely needed, so they appear only with
`--evidence`.

`--location` lists the findings and not-evaluated outcomes located in a
storey or space, named by its name (`--location "Level 1"`) or as `--object`
names an object; it needs a result from `check --locate`. An outcome whose
location is unresolved is listed too, since it may lie there. Located
entries print their place:

```text
[finding] error ducts-through-walls  #29 IFCDUCTSEGMENT …
    location: storey Level 1 (#100); space 101 (#39)
```

**Entries without an object.** A finding or not-evaluated outcome can be about
a whole source ("the model has no space") or the whole project rather than one
object; in the result it has a `source` and no `object_id`, or neither. A
listing entry then carries `scope` (`source model.ifc` or `project`) instead of
`object`, and the text listing prints it in parentheses:

```text
[finding] error spaces-exist  (source model.ifc)
    no object matches the selection in source `ifc-step:model.ifc`; required at least 1
```

`--object` also accepts a source document name (`model.ifc`) and lists the
entries about that source. The summary never gives such an entry as an example,
since every example is an object to drill into; its message names the source.

**Tables.** Measured values rules report (storey heights, areas, ratios; see
[Report tables](./capabilities.md#report-tables)) form the `table` section of
the summary: one group per table, its rule, name and row count, its columns
with units, and example objects. They never change the status. When the
first group is not a table, `next` suggests `--section tables`. The listing
has one entry per row: its `level` is the table's name, its message the
row's values with units, and `--rule` and `--object` filter rows as they
filter findings:

```text
table:
       2  levels   storey-heights
          columns: elevation (m), height (m)
          e.g. #101 IFCBUILDINGSTOREY …; #102 IFCBUILDINGSTOREY …

$ axioval report r.json --section tables --rule storey-heights
[table] levels storey-heights  #102 IFCBUILDINGSTOREY 0000000000000000000102
    elevation 3 m · height 3.5 m
```

Numbers are shown to six decimals, an interval as `lower..upper`; the saved
JSON keeps them in full.

**Decisions.** A listed finding prints its `id` and, when `check
--decisions` carried one over, its decision:

```text
[finding] error wall-reference-required  #101 IFCWALL 0000000000000000000011
    missing exact property axioval:example.ifc.reference
    id: 5c1f0c9e-6a0b-5d53-9a8e-2f3b8f6c1d20
    decision: accepted by A. Reviewer on 2026-09-27T08:00:00Z: agreed with the architect
```

The summary then counts them (`decisions: 1 accepted · 0 rejected · 0 open ·
3 undecided · 0 changed · 1 stale`), and stale decisions form the
`stale-decision` section, grouped by rule and status and listed with
`--section stale-decisions`. `--decision accepted|rejected|open|undecided|changed`
lists only findings with that decision (`changed`: decided and changed
since).

A grouped table, such as a [takeoff](./capabilities.md#information-takeoff),
names its group columns in the summary (`grouped by type, storey; columns:
count, sum_net_side_area (m²)`), and each listed row starts with its group
values in brackets:

```text
[table] takeoff wall-takeoff  (source model.ifc)
    [Basic Wall 200] [EG] count 3 · sum_net_side_area 36.5 m²
```

**CSV.** `--csv` prints one table as CSV (RFC 4180 quoting, `\n` line
ends) instead of a view: the table `--rule` and `--table NAME` select, or
the result's only table. Selecting none or several fails (status 1) and
names every candidate as `--rule R --table NAME`. The columns are `scope`
(`project`, `source …` or an object id), the group columns, then each text
column as it is and each number or quantity column as `<id>_lower` and
`<id>_upper`, the unit in brackets (`sum_net_side_area_lower [m²]`): an
exact value fills both with its full precision, an unknown one neither.

```text
$ axioval report r.json --csv --rule wall-takeoff --table takeoff > takeoff.csv
```

`--csv` cannot be combined with `--json`, `--section`, `--code`, `--object`
or `--location`.

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
geometric rules (clash, containment, distance, contact, space, plan-area, facade-area, slab-stack,
storey-height, exit-separation, corridor-end-openings, escape-route, space-distance, counterpart-coverage, parking-bay, wall-spacing, accessible-route, body-extent, triangle-count, space-boundary-coverage, stair-geometry, ramp-geometry, component-clearance, centre-line-distance, component-visibility, door-swing, effective-coverage, local-circulation and free-space checks) can run, and registers the walkability
and metric-routing services. It is off by default because meshing costs time a purely semantic ruleset
does not need. Without it, geometric rules report `missing-service` (status 4),
never pass, and the summary suggests `--geometry`.

The CLI meshes each product's net body, with openings subtracted, using
`ifc-geometry` and hands the meshes to the Axiolid geometry services, one set
over every model, bound to every model's snapshot. Evidence about one object
cites that object's model; set-level evidence (free space, guards, envelope,
unallocated regions) cites the first model in source order. That
bridge lives in the CLI, not in an adapter, because the IFC and Axiolid
adapters must not depend on each other. Every object ends in one of four
states:

| State | Meaning | Effect on geometric rules |
|---|---|---|
| exact | every face is planar (polygonal extrusions, including steel sections without fillets or rounded edges, faceted B-reps, meshes, booleans of these), so the mesh is the shape | measured as exact |
| tessellated | some face is curved; the mesh is within 1 mm of it | measured as approximate, never exact |
| no body | the object occupies no material: spatial structure, openings, annotations, grids, ports, structural analysis items, non-products | ignored as an obstacle |
| unmeasured | a physical product that could not be meshed, including one without a Body representation | measurements it could affect are not evaluated |

The planarity check is conservative. Anything it does not recognise counts as
tessellated, which only loses exactness, never presents an approximation as
exact.

Space validation also needs roles and storeys: `IfcSpace`, `IfcSlab`, `IfcRoof`
and `IfcBuilding` give roles, and the spatial tree gives each object's storey.
An element the file places twice, or anything under a structure aggregated
twice, gets no storey rather than a guessed one. The slab and roof roles are
only the default cap elements: a rule's `top_cap_elements` or
`bottom_cap_elements` selector replaces them for that cap, as
`boundary_elements` and `intersection_elements` replace the default
bounding and intersecting bodies.

Space-boundary coverage takes every `IfcSpace` and every `IfcRelSpaceBoundary`
(and subtype) naming one. Each boundary's `ConnectionGeometry` surface is
lowered in the frame `ifc-geometry`'s `product_representation_frame` gives the
space's body (the body context's world coordinate system above the space's
placement), exactly as the body is, then meshed; a curve-bounded plane takes
the frame on its basis plane only, so its boundaries stay in the plane's
parameters (openbimrs/ifc#163), and with straight boundaries it is exact. A boundary with no connection
geometry, a point, curve or volume connection, or a surface the lowering or
compiler refuses is unmeasured, and its space is not evaluated. Surfaces, face
surfaces (`IfcFaceSurface`, `IfcAdvancedFace`) and face-based surface models
are lowered through `ifc-geometry` (openbimrs/ifc#155).

The result's `geometry` field records the counts and every unmeasured object
with its reason. The summary prints a `geometry:` line and groups unmeasured
objects by reason; `axioval report result.json --section geometry` lists them.

On a real 6 MiB IFC4 model, 951 bodies mesh exactly, 20 as tessellations
(round columns) and none fail, and a wall-against-wall clash check finds the
overlapping wall joints (0.05 m and 0.10 m) in under a second.

Shelf capacity reads each space's doors and openings through the rule's
`access_path`, such as `axioval:derived.adjacent-space`, and places their
clearances from their bodies; the bridge hands the linear-quantity service
the void of every bodiless `IfcOpeningElement`, so a doorless opening's
clearance is placed too. An opening whose void could not be meshed leaves the
spaces it opens into not evaluated.

Effective coverage continues effects through doors and openings the same
way: the bridge hands the plan-area service the exact void of every bodiless
`IfcOpeningElement`, so a sprinkler's travel or sight passes a doorless
opening into the next room. A void that is not exact, or could not be
meshed, is never passed through, and a coverage it might widen keeps its
upper bound at the whole room.

Facade areas (`measure: facade`) look for the interior in every `IfcSpace`:
a face looking into a space is an inner face. A room the model does not
represent by an `IfcSpace` looks like the outside, so facade areas are only as
good as the model's spaces. Which walls are external is the rule's selection,
typically `IsExternal` in `Pset_WallCommon`.

A group (`IfcGroup`: a zone, a system) has no body, but plan-area rules need
its footprint, e.g. `plan-coverage` of spaces against the zones that model
fire compartments. The CLI reads each group's members from
`IfcRelAssignsToGroup` and declares them, so a group's footprint is the union
of its members' footprints. A member without a body or whose body could not
be meshed, a group that groups nothing, and a membership the relationship
service refuses make the footprint unavailable: rules that need it report not
evaluated, never measure the group as zero.

Relationships the model does not state can be derived from geometry. With
`--geometry`, every relationship capability accepts the identities
`axioval:derived.contained-in-space`, `axioval:derived.adjacent-space`,
`axioval:derived.overlapping-group-space`, `axioval:derived.spans-level` and
`axioval:derived.intersects` (see [typed host services](./services.md)) as its
`relationship` or as a
`path` step; every other identity still goes to the IFC relationship service.
Every `IfcSpace` is a space, and every `IfcDoor`, `IfcWindow` and
`IfcOpeningElement` an opening. Every `IfcBuildingStorey` is a level whose
band runs from its placement's height up to the next storey's of the same
parent; a storey whose placement cannot be read or is tilted, or that shares
its height with a sibling, has no band, and a request it could answer
refuses. With `relationship: axioval:derived.spans-level` and `direction:
backward` from each storey, a storey's `plan-area` or `area-ratio` counts a
two-storey atrium in both storeys. With `host_path:
["axioval:derived.intersects"]`, `opening-zone` judges a duct or pipe in every
beam or wall its body passes through, with no void modelled. An opening element has no body, so the CLI
meshes its void separately for the derivation alone; a void that cannot be
meshed makes the derivation refuse for it. For example, `related-count` with
`relationship: axioval:derived.contained-in-space` and `direction: backward`
counts the furniture in each space of a model that states no spatial
containment, and with `axioval:derived.adjacent-space` forward from each door
counts the spaces it connects. Without `--geometry`, the IFC relationship service
refuses a derived identity, so its rule is not evaluated.

Walkability takes its surfaces, entrances, obstacles and connectors from each
request; the CLI hands over only the voids of opening elements. Metric
routing has no selection in its request, so the CLI declares every `IfcSpace`
a walkable surface, every `IfcDoor` and opening element a portal, and every
`IfcStair`, `IfcStairFlight`, `IfcRamp`, `IfcRampFlight` and
`IfcTransportElement` a vertical connector; every other body obstructs,
including a window filling an opening. IFC states no door clear width the CLI
trusts (a door's overall width includes its lining), so none is declared: a
door bounds a route's width from above only, and a route through a door that
could pass is undecided rather than reachable. A rule can state door clear
widths itself: `accessible-route` reads them from its `clear_width_property`
and sends them with its walkability request. See
[Walkability topology](./walkability.md) and
[Metric routing](./metric-routing.md).

External-wall validation compares the objects a model declares external with
the objects on the envelope of a set of bounding spaces. IFC does not say which
spaces make up the conditioned volume, and the CLI does not guess: the rule
selects them. `--geometry` alone registers the envelope service. The
`all-spaces` derivation is bounded by the rule's `bounding_selector` (for
example every `IfcSpace`), and `gross-area-groups` by the members of the groups
its `gross_area_group_selector` selects, reached along `gross_area_group_path`
(for example `IfcZone`s by name, then `IfcRelAssignsToGroup:forward`). One rule
may run both; see [Capability model](./capabilities.md).

- Bounding spaces must reach the envelope's outer faces, as gross-area spaces
  do. An object is on the envelope when its plan footprint overlaps them and
  reaches their outline, so a wall wholly inside is internal.
- An object's declaration is its `IsExternal`, from whichever property set
  states it: `true` is external, `false` internal. An object with no
  `IsExternal`, or with conflicting ones, is undeclared: its rule reports not
  evaluated, never "internal". So is an object whose body could not be
  meshed, since its membership is unknown. A bounding space without a body
  makes that derivation's envelope unavailable.
- `--envelope-zone NAME` is gone. It named one zone whose spaces bounded both
  derivations; a rule now states the same with `gross_area_group_selector` and
  `gross_area_group_path`, or with a `related` bounding selector. Passing it is
  a usage error (status 2).

Guard checks need to know which surfaces are walkable, and IFC has no single
concept for it: floor, landing and roof slabs, stair and ramp flights, space
floors and balconies each carry a policy choice. The CLI does not guess.
The walking-surface profile is the horizontal-guard rule's own selector, so
it is visible and reviewable in the ruleset. For example, select `IfcSlab`
with a `PredefinedType` of `FLOOR` or `LANDING`. Every selected object's
exposed edges are measured:

- A selected object without a measurable body is reported not evaluated, never
  "no edge to guard".
- Findings name only selected surfaces.
- While any body in the model could not be meshed, guard rules are not
  evaluated, since the unmeasured body may be the rail that guards an edge.

A mesh does not say whether a body is a railing or a cupboard either. The
rule's optional `barrier_selector`, `landing_selector` and
`climbable_selector` name which objects may act as barriers, landings and
climbing aids, for example `IfcRailing` and `IfcWall` as barriers. Only their
members are measured for that role. Without a selector, any body near the
edge counts for its role, so a cupboard standing along an edge can pass as
its barrier.
