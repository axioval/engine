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

With a draft, the rulesets are its context and the draft is checked
within them, as an editor needs while a rule is built (see
[Rule drafts](drafts.md)):

```bash
axioval validate --definitions d.json --ruleset r.json --rule draft.json [--model m.ifc [--geometry]]
axioval validate --definitions d.json --ruleset r.json --expression e.json --into RULE [--parameter requirement]
axioval validate --definitions d.json --ruleset r.json --serve
```

`--rule` (a rule as a ruleset writes it; `-` reads standard input)
replaces the context's rule of the same id or joins the first ruleset;
`--expression` is placed as `--parameter` of the rule `--into`. Prints
`{"valid", "rule", "diagnostics"}` as JSON; with `--model`, a valid draft
is also run, and `dryRun` lists how the drafted rule judged each object:
every expression's trace (`passed`, `failed` or `notEvaluated`) and the
rule's findings and not-evaluated outcomes. `--serve` reads one request
per line (`{"rule": …}` or `{"expression": …, "into": …}`) and answers
each with one line, until standard input ends. Exit status: 0 valid, 3
the draft is refused, 1 the context cannot be read.

Every command reading packages (`validate`, `check`, `export`) loads the
table files a ruleset's rules or a definition package's defaults name
(`tableFile` parameters, see
[Tables from data files](capabilities.md#tables-from-data-files)) from the
directory holding that package file. A file that is missing, outside that
directory, of another digest or not fitting its declared columns fails the
command (status 1) before any model is read.

### MCS package archives

Wherever a command takes `--ruleset` or `--definitions` (`validate`,
`check`, `export`, `catalogue`), the file may also be an
[MCS package archive](./mcs-archives.md): a file named `.mcs`, or one
opening as a ZIP archive, is read as one; anything else is read as JSON.

```bash
axioval check --model building.ifc --ruleset rules.mcs --definitions library.mcs
```

The archive's own manifest says what it is. A `ruleset` archive goes to
`--ruleset` and carries the definition packages its rule set binds, so it
needs no `--definitions` of its own; a `definitions` archive (a template
library) goes to `--definitions`. The other way round is refused (status 1).
A rule set's `definitionPackages` resolve from the packages its archive
carries and from every `--definitions` given; a package carried and given
is kept once when both copies are equal and refused when they differ, and
an ID neither gives fails as it does for JSON (status 1). Table files are
read from the archive's package root. An archive the MCS reader would
refuse (an unsafe or duplicate member, a size over its limits, a member
not matching the inventory, an identity the metadata and the manifest do
not share) fails the command before any model is read. `check --ruleset
rules.mcs --definitions library.mcs` reports exactly what the same check
with the normalized JSON the archives carry reports.

## `axioval catalogue`

Prints the [authoring catalogue](./catalogue.md): every capability with its
parameters, the measured values, expression node kinds, operators,
selector kinds, relationships and units, labelled in English and German,
as versioned JSON a rule editor is generated from. `--definitions` (repeat
for several; JSON or a `definitions` archive) adds each package's concept
vocabulary; `--output FILE`
writes to a file instead of standard output; `--locale de` writes every
text in German only, listing on stderr any that fell back to English.
Exit status: 0 written, 1 nothing written (an unreadable package, an
unsupported locale, or a capability without texts).

## `axioval check`

```bash
axioval check --model building.ifc[:DISCIPLINE] [--model other.ifc[:DISCIPLINE] ...] \
  [--discipline-map FIELD:PATTERN=DISCIPLINE ...] \
  (--definitions definitions.json --ruleset ruleset.json [--ruleset other.json ...] \
   | --ids rules.ids [--ids-filter selector.json]) \
  [--relations RELATION=FILE[#SHEET] ...] [--catalogue CATALOGUE[@VERSION]=FILE[#SHEET] ...] \
  [--zones [ACCURACY=]FILE ...] \
  [--terrain KIND@ACCURACY=FILE[#SURFACE] ... --terrain-in model|map] \
  [--geometry [--no-exact-boundaries]] [--locate storeys|containers|geometry] \
  [--rule-status] [--rule-timings] [--timings] [--threads N] [--report result.json] \
  [--decisions decisions.json | --decisions-from reviewed.bcfzip] \
  [--summary [--top N]] [--bcf issues.bcfzip] [--xlsx report.xlsx] \
  [--html report.html [--html-template template.html] [--html-title TEXT]] \
  [--bcf-author NAME] [--bcf-date 2026-09-26T10:00:00Z] [--bcf-version 2.1|3.0] \
  [--bcf-subject-color HEX] [--bcf-related-color HEX] [--bcf-no-color] \
  [--bcf-isolate] [--bcf-section-box] [--bcf-frame-witness] [--bcf-snapshots]
```

Runs the ruleset over one or more IFC2X3, IFC4 or IFC4X3 STEP models. Each
model is one source, named by its file name, so a result does not depend on
the directory it was checked from.

A model may also be ifcXML (any file that starts as an XML document), in the
buildingSMART XSD configuration of IFC4 or IFC4X3 or in the `ifc-xml` codec's
own layout, read into the same model as its STEP form and so giving the same
result apart from its source, `ifc-xml:<file name>`. A document in neither
layout, or stating a name or value its release does not declare or admit, is
refused with status 1 (see [ifcXML](./adapters.md#ifcxml)).

A model may also be an ifcZIP archive (`building.ifczip`, or any file that
is a zip archive) holding exactly one `.ifc` member, which is read exactly as
the plain file would be (see [ifcZIP](./adapters.md#ifczip)). Its source is
named by the archive and the member, `building.ifczip/building.ifc`; apart
from that name the result is the plain file's. An archive with no model, with
several (an `.ifcXML` member counts), with only an IFC-XML model, or with a
member path that is absolute, climbs out with `..` or is a symbolic link is
refused with status 1 and nothing is written.

### IDS documents

`--ids rules.ids` checks against a buildingSMART IDS 1.0 document instead of
packages. It is translated in memory by [`axioval-ids`](./ids.md), exactly as
`axioval ids translate` writes it, and run as one ruleset whose rule ids are
the translator's (`spec2.facet1`). It excludes `--definitions` and
`--ruleset`.

The document is [audited](./ids.md#audit) against the IFC schemas of its
listed releases first. A document with any audit error cannot work as
written and checks nothing: the check exits 1 before reading the model,
writes nothing, and lists every finding with its specification (from 1),
code, path and release:

```text
axioval: rules.ids: the IDS document cannot work as written, so nothing is checked:
ids:   error [entity-name-case] specification 2 at applicability/facets[0]/name: "IfcWall" is not upper case; IFC entity names are written as "IFCWALL"
```

An audit warning (`pattern-unverified`, a pattern the audit does not
evaluate) refuses nothing. It is listed on stderr
(`ids: rules.ids: audit warning [pattern-unverified] specification 1 at …`)
and in the `ids` field's `warnings`, and never changes the exit status.

Only complete specifications run. A specification with any translation gap
runs none of its rules, since part of it would report as met what was never
wholly checked, and is listed on stderr with every gap, as the conformance
corpus lists them:

```text
ids: rules.ids: specification 1 "Long names" is not checked:
ids:   applicability facet 1: a name restriction with facets other than an enumeration and patterns is not translated
ids: rules.ids: 1 of 2 specification(s) not checked
```

`--ids-filter selector.json` restricts every specification to part of the
model, such as the walls of one storey: the JSON selector, written in IFC
names, is combined with each specification's applicability through `allOf`
(see [Prefilter](./ids.md#prefilter)), so objects it leaves out are not
checked at all. It needs `--ids`, is recorded as the `ids` field's
`filter`, and a selector naming a rule (`ruleOutcome`) fails with status 1.
`ids translate` takes it too.

The other specifications run as usual. The result's additive `ids` field
names the document and lists every specification in order with its number,
name, the rules it ran as, and its gaps, and the audit's `warnings` (each
with `specification`, `code`, `severity`, `path` and `ifc_version` when it
holds for one release only, and `message`; left out when there are none). A
check with a specification that did not run never exits 0: with no finding
it exits 4.

### Relations supplied beside the model

A ruleset may declare a relation whose pairs are project data, `by:
supplied` (see [Declared relations](./derived.md#declared-relations)), so
one rule package serves every project. `--relations serves=pairs.csv`
supplies its pairs for this check: a CSV file, or one sheet of an xlsx
workbook (`--relations serves=pairs.xlsx#Pumps`), read by the columns the
relation declares, as a [table file](./capabilities.md#tables-from-data-files)
is. Repeat it for several relations. The relation id ends at the first `=`;
`#SHEET` is read only after a path ending in `.xlsx`.

Before any model is read, the check fails with status 1, writing nothing,
when a file names a relation no ruleset declares, one whose ruleset states
its own pairs, or one already given a file, or when the file cannot be read,
is larger than 10 MB, or its header or rows are refused (an undeclared or
missing column, a blank or non-text cell). A supplied relation given no file
relates nothing surely: every rule walking it is not evaluated, and
`integrity` lists it as the warning `relation-pairs-not-supplied`. A pair
naming an object the model does not hold is reported as
`relation-object-unknown`, as a listed pair is.

The result's additive `relation_files` records each file supplied, so the
run can be reproduced; every pair's evidence cites the same digest:

```json
"relation_files": [
  { "relation": "serves", "file": "pairs.csv", "sha256": "9f2c…", "pairs": 42 }
]
```

### Catalogues supplied beside the model

A ruleset may declare a [catalogue](./expressions.md#catalogues) of
material or product data whose entries expressions join to objects by a
key, such as each material's thermal conductivity by its name.
`--catalogue materials@2026-1=materials.csv` supplies its entries for this
check: a CSV file, or one sheet of an xlsx workbook
(`--catalogue materials=materials.xlsx#Data`), read by the columns the
catalogue declares, as a
[table file](./capabilities.md#tables-from-data-files) is, with the
catalogue's version after an optional `@`. Repeat it for several
catalogues. The catalogue id ends at the first `@` or `=`.

Before any model is read, the check fails with status 1, writing nothing,
when a file names a catalogue no ruleset declares or one already given a
file, when the version is blank, or when the file cannot be read, is larger
than 10 MB, or its header or entries are refused (an undeclared or missing
column, a cell not of its column's kind, an entry naming no key or a key
another entry names). A catalogue given no file answers no key: every
object whose expression reads it is not evaluated, and `integrity` lists it
as the warning `catalogue-not-supplied`.

Every value read cites the catalogue, the version and the entry (see
[Catalogues](./expressions.md#catalogues)), and the result's additive
`catalogue_files` records each file supplied:

```json
"catalogue_files": [
  { "catalogue": "materials", "version": "2026-1", "file": "materials.csv",
    "sha256": "4be1…", "entries": 3 }
]
```

### Results of external calculations and model revisions

A catalogue may hold [results of external
calculations](./expressions.md#results-of-external-calculations), such as
the R'w per wall a sound insulation calculation computed, each stating the
model revision it was computed on. `--catalogue sound=rw.csv` supplies
them as any catalogue, and the check identifies the revision of each
checked model by the SHA-256 of its file as given (`sha256:<hex>`; for an
ifcZIP archive, the archive's bytes), plus the revision
`--model-revision` states for it: `--model-revision rev-12` for the one
model checked, `--model-revision structure.ifc=rev-12` naming a model by
its file name among several. A result is read only for the objects of a
model whose revision it names.

Every supplied result the check will not read is an `integrity` warning,
located at `axioval:catalogue;id=<catalogue>#entry=<key>`:
`calculation-result-stale` for one computed on a revision no checked model
has, `calculation-result-untraceable` for one stating no method, report,
date or revision. Each object reading such a result is not evaluated, the
reason saying why. A `--model-revision` naming no checked model, stated
twice for one, or bare beside several models stops the check with status 1.

### Context zones supplied beside the model

Parcels, zoning areas, building limits and building lines are not part of
a model. `--zones zones.geojson` supplies them for this check as
[context zones](./context-zones.md): a GeoJSON feature collection naming
its reference system in `crs` (`axioval:model` for the model's own plan
coordinates, or a map system such as `EPSG:25832` that the model's map
conversion names), each feature a zone with an `id`, a `kind`, an
`accuracy` in metres and a polygon, multipolygon or line string, and
optionally the `values` it states, such as the zone of a snow load zone
map (see [Location-dependent design
data](./context-zones.md#location-dependent-design-data)).
`--zones 0.05=alkis.xml` supplies a GML document instead: cadastral
parcels and public traffic areas (see
[Cadastral parcels](./context-zones.md#cadastral-parcels-gml)), or a
zoning plan rooted in `XPlanAuszug` with the values its areas state (see
[Zoning plans](./context-zones.md#zoning-plans-xplangml)). GML states
no accuracy of its boundaries, so the argument states it before the first
`=`, in metres (a trailing `m` is allowed); a GeoJSON file given an
accuracy, and a GML document given none, are refused. Repeat it for
several files; a zone id may be stated once only.

Before any model is read, the check fails with status 1, writing nothing,
when a file cannot be read, is larger than 64 MB, names no reference
system, states a zone without an id, kind, accuracy or readable geometry,
states a feature or geometry outside a reader's subset, or states an id an
earlier file states. A rule reading context zones when no file was given
leaves every value reading them not evaluated, and `integrity` lists the
warning `zones-not-supplied`; a zone the rules name (in a measured value
or the rule's text parameter it binds) that no file states is the warning
`zone-not-supplied`, located at `axioval:zone;id=<id>`, and a kind they
read as the zone an object stands in (`stands_in=<kind>`) that no file
states is `zone-kind-not-supplied`, at `axioval:zone;stands_in=<kind>`.

Once the models are read, each file is compared with each model: whether
the model's coordinate system places the zones (`reference`: `model` for
zones in the model's own coordinates, `matches` where the model's map
conversion maps onto the zones' system and places them, `refused`
otherwise, with the `reason`), and, with `--geometry`, whether the placed
zones lie over the model (`coordinates`: `overlap` where their extent meets
the extent of the model's measured bodies, `apart` with the `gap_metres`
between the two, `unchecked` with the `reason` otherwise). A refused
placement is the integrity warning `zone-reference-mismatch`, zones lying
apart `zones-apart-from-model`, both located at
`axioval:zone;sha256=<digest>`.

The result's additive `zone_files` records each file supplied, its format,
reference system, the accuracy its argument stated, its zones and those
placements, so the run can be reproduced; every value read from a zone
cites the same digest:

```json
"zone_files": [
  { "file": "alkis.xml", "sha256": "4b1e…", "reference": "EPSG:25832",
    "zones": ["0001", "0002", "0003", "S1"], "format": "cadastre-gml",
    "accuracy_metres": 0.05,
    "placements": [
      { "source": "ifc-step:model.ifc", "reference": "matches", "coordinates": "overlap" }
    ] }
]
```

### Terrain beside the model

`--terrain original@0.05=dgm.xml --terrain-in map` supplies a terrain
surface as context (see [Terrain as context](./terrain.md)): which ground it
is, `original` or `finished`; its vertical accuracy in metres (a trailing
`m` is allowed), the most any of its elevations may differ from the true
ground, which LandXML does not state and the argument therefore must; and a
LandXML file holding one TIN surface, or the one named after its last `#`
(`dgm.xml#Existing`). Repeat it for the other ground. `--terrain-in` is
required with it and says whether the files are in the models' own
coordinates (`model`) or in the map each model's stated map conversion maps
onto (`map`).

Before any model is read, the check fails with status 1, writing nothing,
when a file cannot be read, is larger than 256 MiB, states no or an unknown
linear unit, holds no such surface or several without one being named, or
its surface is refused (not a TIN, a face naming a point it does not state,
no triangle with plan area), and when one ground is given twice. An
argument without an accuracy, and `--terrain` without `--terrain-in`, are
usage errors (status 2). A ground the rules measure against (any
`above_terrain` or `mean_terrain_height` naming it, or both grounds where a
rule binds `terrain` from a parameter) that no file supplies is listed in
`integrity` as the warning `terrain-not-supplied` (locator
`axioval:terrain;kind=<kind>`); every such value is not evaluated.

The result's additive `terrain_files` records each surface supplied:

```json
"terrain_files": [
  { "terrain": "original", "file": "dgm.xml", "surface": "Existing",
    "sha256": "4be1…", "accuracy_metres": 0.05, "frame": "map",
    "crs": "EPSG:25832", "points": 1250, "triangles": 2396 }
]
```

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
The CLI never moves one model onto another: each is meshed in its own model
coordinates, which are one frame only when the models share a coordinate
system. Every model is therefore compared with the first `--model` (world
frame within 1 mm and 0.01°, and the same map conversion, or none in
either): a model that does not share it, or whose coordinate system cannot
be read, has every physical object unmeasured with the reason (`not in the
coordinate system of ``arch.ifc``: map offset moved by 1.0000 m`), so its
clashes and distances with the other models, and its own geometric checks,
are not evaluated rather than measured in the wrong place. Check such a
model alone, or fix its georeference; a `coordinate-consistency` rule
reports the difference as a finding (see [Coordinate
consistency](./capabilities.md#coordinate-consistency)).

Many delivered models state no frame that can be compared: their only
representation context is a `Plan` or `Design` one rather than `Model`, or
only one of two states a map conversion, or a file states its frame
ambiguously (several `Model` contexts on different placements). When the
models are known to overlay as delivered, `--declare-shared-frame` (with
`--geometry`) says so: every such model is measured in the first model's
frame. The declaration covers missing statements only, never contradicting
ones: a model whose stated world frame or map conversion differs beyond the
tolerance, or whose coordinate system cannot be read or compared, stays
unmeasured as without it. The result's additive
`geometry.declared_frame_sources` names every model measured on the
declaration, stderr notes each (`note: … is measured in the first model's
frame as declared (--declare-shared-frame), not as its coordinate system
states`) and the summary counts them, so a reader can tell an alignment
declared from one read. Models placed far out are still measured about one
local origin, the declared ones shifted by it as the first is.

### Derived models

Models are often derived from one another: a structural model exported
from the architectural one keeps copies of its walls, slabs and columns,
and one authoring model exported several times with different exchange
settings gives several files of the same objects. Checked together, every
rule would see each such object twice: a per-object rule reports the wall
once per file, and a clash rule reports the copy against its original and
a duct crossing the wall twice. `--derived-from DERIVED=ORIGINAL` (with
`--geometry`, models named by file name, repeatable) declares that DERIVED
was derived from ORIGINAL. Each object of DERIVED matched to an object of
ORIGINAL is then superseded by it: no rule's subject or counterpart, its
findings reported once through the original, rule summaries (`checked`)
and the BCF, spreadsheet and HTML outputs counting only evaluated objects.
The result's report lists every superseded object in `superseded` with the
object superseding it and the reason (`kind` and `text`), `geometry.superseded` each pair with
how it was matched (`by-id`, `by-geometry`), `geometry.derived_unmatched`
how many objects of the derived models stay evaluated by cause, and the
summary says both (`derived: 1 superseded by their originals · 4 still
evaluated (2 different-body, 1 no-body, 1 no-counterpart)`). All of it is
additive and absent without the flag, when the result is unchanged.

Matching (`--derived-match`, default `id`):

- `id`: an object with the original's `GlobalId` and class is its copy,
  if its body is confirmed to be the original's.
- `id-or-geometry`: as `id`, and an object with no such counterpart (an
  export that does not keep ids) is paired by geometry alone: the same
  class, the same body, and exactly one candidate on each side, found
  through a grid over the bodies' boxes, never by comparing all pairs.

Bodies are compared in the shared world frame by what the geometry
service measures, never by representation or triangles, since another
export may state one body as an extruded profile, a B-rep or triangles.
Within `--derived-tolerance` metres (default 0.001), widened by each
mesh's certified chord deviation `d`: every corner of the two bounding
boxes within `t + d1 + d2`; the mesh areas within `2π (a + b + c)` times
that, what offsetting a convex body by it adds to its area at most (`a`,
`b`, `c` the larger box's extents); and the certified volume intervals
within `t` times the larger area. The fingerprint only filters: two walls
of one size, each with one opening of one size in different places, share
box, area and volume. A match is confirmed only by the two surfaces'
certified Hausdorff distance being within `t`: measured by the geometry
service between two exact meshes or two exact boundaries, and otherwise
between the two meshes by the kernel's certified mesh distance (each
mesh's triangles subdivided until the interval is within `t / 4` or a
budget runs out, every bound rounded outwards), widened by `d1 + d2` to
bound the true surfaces. Certainly beyond `t` is `different-body`; an
interval left open is `undecided`. A tessellated copy is therefore
superseded only where `t` exceeds the meshes' chord deviations together
(a few millimetres for the default 1 mm chord budget); below that it
stays evaluated.

The safe side is double reporting, never hiding: an object whose identity
with an original cannot be established stays fully evaluated, counted by
cause: `no-counterpart` (no object with its id, nor, by geometry, its
body), `different-class`, `different-body` (moved or reshaped beyond the
tolerance), `no-body` (it or its counterpart occupies no material, such
as a storey), `unmeasured` (either body unmeasured, including a model not
in the first model's frame), `undecided` (a volume that cannot be
bounded, a distance interval left open) and `ambiguous` (several
candidates by geometry). A model declared derived from itself or from two
models, or derived and also the original of another, fails with status 1
before anything is written; declare every derived model from the original
directly.

With `--geometry`, a body whose construction is exact also gets its exact
boundary, built from the graph its mesh is compiled from: an extrusion or
revolution of a rectangle, circle, section or line-and-arc profile (an
extrusion of an ellipse too), or a disk swept along one segment or one
arc, under any rigid placement (tilted, horizontal, turned, mirrored or
mapped), less its openings where they are extrusions and clipped by roof
planes (`IfcBooleanClippingResult`; exact where the kernel decides nothing
within its tolerance, else widened by what it reports, see
[Clash](./clash.md#axiolid-measurement)), and a body of several such items
(a column on its footing). A boundary is
registered only when its
extent agrees with the mesh's within the chord deviation, and only when
some curved body has one (two planar bodies are measured exactly anyway).
Distances between such bodies are then certified to a micrometre instead
of being widened by the 1 mm chord deviation: a round column 0.8 m from a
wall clears a 0.79999 m minimum that its mesh alone leaves open. The
result's `geometry.exact_boundaries` counts them (left out when zero), as
do the stderr line and the summary.
Building them is cheap; certifying costs time per pair of curved bodies
near each other, as the kernel's branch and bound refines each certified
distance to a micrometre. On the buildingSMART example models a check takes
under 0.05 s either way. On a 29 MB model with 56 curved bodies, 135
bodies get boundaries (50 when only vertical extrusions were built), and a
column-to-wall distance rule took 920 s against 886 s meshing only
(unoptimised build on a loaded machine; meshing the model dominates).
`--no-exact-boundaries` meshes only, for a quick first pass. `axioval compare`
builds them only with `--geometry-mode mesh`, where the surface distance reads
them (and `--no-exact-boundaries` turns that off too).

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
It also lists, as `relation-object-unknown` warnings, every listed or
supplied pair of a ruleset's
[declared relations](./derived.md#declared-relations) naming an object the
model does not hold, or several, and as `relation-pairs-not-supplied`
warnings every supplied relation given no `--relations` file; the rules
following them are not evaluated, and the exit status follows from them.
As `catalogue-not-supplied` warnings it lists every
[catalogue](./expressions.md#catalogues) given no `--catalogue` file; the
objects whose expressions read it are not evaluated.
It lists as `zones-not-supplied` a ruleset reading context zones given no
`--zones` file, as `zone-not-supplied` every zone it names that no file
states, as `zone-kind-not-supplied` every kind it reads as the zone an
object stands in that no file states, and as `zone-reference-mismatch`
and `zones-apart-from-model` a file whose zones a model cannot place or
which lie apart from it (see
[Context zones](#context-zones-supplied-beside-the-model)).
With `--geometry` it lists, as `shape.no-representation` warnings, every
physical product with no shape representation and no parts, and as
`shape.self-intersecting-face` warnings every product unmeasured for a face
whose boundary crosses or runs back along itself, as `shape.voided-body`
warnings every product whose openings remove its whole body, as
`shape.invalid-geometry` warnings every product whose geometry is
geometrically invalid as written (a whole unmeasured only through such
parts with its part's code), and as `shape.open-surface` warnings every measured product whose faces, as
authored, leave edges bounding one face only, once each (see
[Geometry](#geometry)).
`objects` maps every object the report names to its kind and, when it has one,
its GlobalId, so a reader can tell what `#4711` is without the model. A
resource object a rule selected by its class (an `IFCMATERIAL`; see
[Entity types and resource objects](./capabilities.md#entity-types-and-resource-objects))
is listed the same way, from the report's `resources`:

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

`--bcf-frame-witness` frames each viewpoint of a finding whose evidence says
where it happens (a [witness](./clash.md#where-a-pair-meets): the closest
pair of a clearance or distance, the deepest point of a clash and the box
two bodies share) at that spot instead of at its objects' bounds, so a duct
through a 15 m wall is shown at the duct; the section box then boxes the
spot, and the description lists each witness with its points and
deviations. Findings without a witness are framed as before. Without the
flag the archive is byte for byte the same.

`--bcf-snapshots` adds a PNG snapshot to each viewpoint with a camera,
rendered from the meshed bodies (see [Snapshots](./sinks.md#snapshots)):
the subject and related objects in their colours, the rest grey unless
`--bcf-isolate`, cut by the section box. It needs `--geometry`, and fails
with status 1 without it. A snapshot is illustrative, never evidence: every
topic with one says so in its description. A viewpoint whose subject has no
mesh gets none, counted in a warning. Without the flag the archive is byte
for byte the same.

`--bcf-version` is `2.1` (default) or `3.0`. BCF 3.0 requires a camera on
every viewpoint, so it needs `--geometry` and bounds for every selected
object; otherwise the run fails with status 1 and nothing is written.

`--xlsx` also writes a spreadsheet workbook (see
[Spreadsheets](./sinks.md#spreadsheets)): a `Findings` sheet listing every
finding and then every not-evaluated outcome, and one sheet per report
table, such as each [takeoff](./capabilities.md#information-takeoff). A
table's numbers are numeric cells in the unit its header names
(`sum_footprint lower [m²]`), an interval is its two bounds, and an
`exactness` column beside each number says `exact`, `bounded` or `not
evaluated`; an unknown value is a shaded blank. The workbook is created at
`SOURCE_DATE_EPOCH` when set, else now, so with it set the same inputs write
byte-identical workbooks.

`--html` also writes the report as one self-contained HTML file (see
[HTML reports](./sinks.md#html-reports)): a cover with the title
(`--html-title`, default `Check report`), the date and the overall status,
then the summary, rules, categories, findings with their objects,
locations and decisions, not-evaluated outcomes, stale decisions and report
tables. Its style is inline and it references nothing outside itself, laid
out for print, so a browser's print dialog (or a headless browser's print
to PDF) makes it a PDF. `--html-template` renders through a template file
instead of the built-in one: HTML with placeholders such as `{{summary}}`,
substituted and never run. A template that names an unknown placeholder,
repeats a section, or leaves out `{{summary}}` or `{{not-evaluated}}` fails
the run with status 1 before any work, and nothing is written. The date is
`SOURCE_DATE_EPOCH` when set, else now, so with it set the same inputs
write byte-identical reports.

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

`--timings` prints how long each phase took on stderr, one line per phase as
it ends, so a run stopped by a time limit still says where its time went:
compiling the packages, reading the models, each phase of `--geometry`
(parsing, meshing, exact boundaries, composing wholes from their parts,
groups, the space, route and derived-relationship services, alignments and
facades, registering the services), each rule by its id, the run as a whole
and writing the outputs, then the total. Envelope declarations (`IsExternal`
of every meshed object) are read only when a rule first asks for envelope
membership, and timed then. The lines are for people and may change; the
result and the exit status are those of a run without it.

```text
timing:      2.077 s read models
timing:      1.872 s geometry: mesh
timing:     41.310 s rule element-clash
```

`--rule-timings` keeps each rule's time in the result instead, beside its
status (it implies `--rule-status`): every rule summary carries a `timing`
with the wall time spent evaluating the rule, the part spent in the host
services it called (geometry measurement, proximity, routes) and the calls
and time per service (see [Rule timings](./refinement.md#rule-timings)).
The summary then lists the slowest rules, at most `--top`, with the service
each spent most in:

```text
slowest rules:
      41.310 s  (host 39.870 s, most in proximity) · 812 checked  element-clash
       0.012 s  (host 0.000 s) · 10 checked  wall-reference-required
```

Timings are diagnostics: without the `timing` fields the result is the one
`--rule-status` writes, and findings, their identities and the exit status
never depend on them.

`--threads N` evaluates each rule's subjects (its spaces, its objects) on
`N` threads (#405). Without it, the environment variable `AXIOVAL_THREADS`
names the count, and without that the machine's available parallelism
does, up to 4 (more threads are taken only when asked for, until a check
budgets its memory, #407); `AXIOVAL_THREADS` set to anything but a positive whole number fails
the run (status 1). The result is the same on any number of threads, byte
for byte: subjects are merged in their order, never in the order threads
finish them, and a run whose expression budget runs out is run again on
one thread (see [Execution](./architecture.md#execution)). Memory grows
with the threads, each holding one subject's measurement at a time; a
memory-bound host lowers `--threads`. Rule timings count every thread's
host calls, so the time a rule spent in a service may exceed its wall
time; its host time is never stated above its wall time.

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

`--decisions-from ARCHIVE` takes the decisions from a BCF 2.1 or 3.0 archive
instead, typically one `--bcf` wrote and a reviewer worked through in
another BCF tool (see [Import](./sinks.md#import)): a topic whose GUID is a
finding's identity decides it when it was closed, resolved, done, accepted
or rejected, or commented on; its comments are carried as the thread. Every topic
that decides no current finding (another tool's topic, a fixed finding's, a
not-evaluated outcome's, or one whose dates have no UTC offset) is listed in
the result's `unmatched_topics` with its GUID, title, status and `reason`
(`no-finding`, `not-evaluated`, `no-guid`, `unreadable` with a `detail`), and
counted on stderr. It conflicts with `--decisions`; an archive that cannot be
read fails the run (status 1) before anything is written.

```bash
axioval check --model rev1.ifc ... --bcf r1.bcfzip       # reviewed elsewhere into reviewed.bcfzip
axioval check --model rev2.ifc ... --decisions-from reviewed.bcfzip --report r2.json
```

Everything is built before anything is written: a failing run leaves no
partial report or archive behind.

### Exit status

| Status | Meaning |
|---|---|
| 0 | Every rule was evaluated and nothing was found |
| 3 | At least one finding |
| 4 | No finding, but part of the check was not evaluated, or an IDS specification did not run: the model has **not** passed |
| 1 | The check could not run: unreadable input, a package that does not compile, a model that does not import, or output the BCF writer refuses |
| 2 | Invalid command-line usage |

A finding takes precedence over incompleteness: status 3 can still come with
not-evaluated outcomes in the report. Automation that only needs pass or fail
treats any non-zero status as a failure.

## `axioval ids translate`

```bash
axioval ids translate rules.ids --definitions definitions.json --ruleset ruleset.json \
  [--ids-filter selector.json]
```

Writes the definition package and the ruleset `check --ids` would run, for
inspection or to check with `--definitions` and `--ruleset`, which then gives
the same report. The ruleset's package id is `ids:` and the file stem in lower
case (`ids:rules`); the definitions' is that and `.definitions`. A
specification with a gap is left out and listed on stderr, as for `check
--ids`. A document the audit refuses is listed with its findings and
nothing is written; audit warnings are listed and refuse nothing. Both
files are written, or neither.

| Status | Meaning |
|---|---|
| 0 | Every specification translated |
| 4 | Written, without the specifications listed on stderr |
| 1 | Nothing written: an unreadable document, one the audit refuses, or an unwritable file |
| 2 | Invalid command-line usage |

## `axioval export`

```bash
axioval export --profile ids --definitions definitions.json --ruleset ruleset.json --out rules.ids
```

Writes a ruleset as another format through an [export
profile](./export.md), chosen by its id. The CLI registers `ids`, which is
[`ids export`](#axioval-ids-export) by another name: same output, same
document, same exit status. A rule the format cannot state exactly is listed
on stderr as not exported; structure or presentation a format reduces
without changing a verdict is listed as degraded
(`<id>: ruleset.json: <path> is degraded: <why>`). An unknown profile exits 1
and names the known ones.

| Status | Meaning |
|---|---|
| 0 | Everything exported, nothing lost |
| 4 | Written, with the losses listed on stderr |
| 1 | Nothing written: no rule the format states, an unknown profile, an unreadable package or an unwritable file |
| 2 | Invalid command-line usage |

## `axioval ids export`

```bash
axioval ids export --definitions definitions.json --ruleset ruleset.json --out rules.ids
```

The same as `axioval export --profile ids`.

Writes the ruleset's alphanumerical rules as an IDS 1.0 document, the
reverse of `ids translate` (see [Export](./ids.md#export)). `--definitions`
repeats for a ruleset that uses several definition packages. Only rules IDS
states exactly are written: a folder `ids translate` wrote, as the
specification it came from, and any other rule as one specification of its
own. Every other rule is listed on stderr with why, and nothing is
approximated. Every specification is audited before it is written, so a
rule that reads as a specification the audit refuses (one listing every
release over a class IFC2X3 lacks) is listed, never written:

```text
ids: ruleset.json: rule walls-clash is not exported: capability axioval:capability.clash has no IDS facet
exported 1 rule(s) as 1 specification(s); 1 rule(s) not exported
```

| Status | Meaning |
|---|---|
| 0 | Every rule exported |
| 4 | Written, without the rules listed on stderr |
| 1 | Nothing written: no rule IDS states, an unreadable package or an unwritable file |
| 2 | Invalid command-line usage |

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
counts), so a re-check can tell whether it changed. `--comment` adds to the
end of the finding's comment thread: deciding again keeps the earlier
comments, so several reviewers can discuss one finding, and the BCF topic
carries the whole thread. A listing shows the latest comment and how many
came before it (`: yes, a lining (+1 earlier comment(s))`). `--assign-to NAME`,
`--due DATE-TIME`, `--priority P` and `--label L` (repeated) record who
deals with it, by when, how urgent and how it is labelled; a later decision
about the same finding that does not restate them keeps them (`--label`
replaces all labels). `check --decisions` carries them to the finding and
the BCF topic's `Priority`, labels, `AssignedTo` and `DueDate`. `--date` is an ISO 8601
date-time with offset, else `SOURCE_DATE_EPOCH` when set, else now, in UTC.
A finding the result does not contain fails the command (status 1) and
nothing is written.

## `axioval bcf push` and `axioval bcf pull`

Exchange a saved result's topics with a BCF API 3.0 server:

```bash
export AXIOVAL_BCF_CLIENT_SECRET=...        # or AXIOVAL_BCF_TOKEN=<bearer token>
axioval bcf push r1.json --server https://bcf.example.com --project P --client-id checker
# reviewers work on the server
axioval bcf pull r1.json --server https://bcf.example.com --project P --client-id checker \
  --decisions decisions.json
axioval check --model rev2.ifc ... --decisions decisions.json
```

`push` maps the result's findings and not-evaluated outcomes onto topics as
`check --bcf` does (without rule labels, which a result does not record) and
creates them in the project, or updates the ones the server has under the
same GUID: pushing twice never duplicates a topic, comment or viewpoint.
For a finding without a decision, the status, priority, assignee, due date
and extra labels on the server are kept. It prints how many topics it
created and updated and how many comments and viewpoints it added.

`pull` reads every topic of the project with its comments and maps them as
`check --decisions-from` maps an archive: a status changed on the server
becomes a decision about its finding, recorded in `--decisions` (created
when missing) with the finding's basis from the result, replacing an
earlier decision about the same finding. Topics that decide no finding are
listed on stderr.

Sign-in: `AXIOVAL_BCF_TOKEN` is used as a bearer token when set; otherwise
`--client-id` with `AXIOVAL_BCF_CLIENT_SECRET` signs in by OAuth2 client
credentials, or `--client-id` with `--device-authorization-url` by the
device flow, printing where to enter the code on stderr. `--token-url`
names the token endpoint when the server's `/bcf/3.0/auth` does not.
Credentials are never arguments and never written to a result or a
decisions file. `https` servers need the CLI built with the `tls` feature
(`cargo install axioval-cli --features tls`, which links the platform's
TLS library); without it they are refused.

| Status | Meaning |
|---|---|
| 0 | Pushed or pulled |
| 1 | Nothing pulled, or the push stopped: an unreadable result, a server that is unreachable, speaks no BCF API 3.0 or refuses the sign-in or a request |
| 2 | Invalid command-line usage |

## `axioval compare`

```bash
axioval compare --base r1/model.ifc --revised r2/model.ifc \
  [--property SET.NAME ...] [--property-set SET ...] [--all-property-sets] \
  [--geometry [--geometry-mode bounds|mesh] [--no-exact-boundaries]] [--timestamps] \
  [--length-tolerance METRES] [--angle-tolerance DEGREES] \
  [--report result.json] [--summary [--top N]] [--bcf changes.bcfzip] [--xlsx report.xlsx] \
  [--html report.html [--html-template template.html] [--html-title TEXT]] \
  [--bcf-author NAME] [--bcf-date 2026-09-26T10:00:00Z] [--bcf-version 2.1|3.0] \
  [--bcf-subject-color HEX] [--bcf-related-color HEX] [--bcf-no-color] \
  [--bcf-isolate] [--bcf-section-box] [--bcf-frame-witness] [--bcf-snapshots]
```

Compares two revisions of one IFC2X3, IFC4 or IFC4X3 model, each a STEP file
or an ifcZIP archive, object by object (see
[Model comparison](./comparison.md)). Objects are matched by `GlobalId`, so a
re-export that renumbers every entity still matches. Each revision is its own
source, named by its file name; when both files have the same name, as two
revisions usually do, the sources are named `model.ifc@base` and
`model.ifc@revised`.

Compared facets:

- kind, classifications and relationships, always; relationships per
  [relationship kind](./comparison.md#relationship-kinds) (containment,
  aggregation, voids, fills, space boundaries, type, group membership,
  connections), so a door moved to another storey is a `containment` change
  and a window moved into another wall's opening a `fills` change;
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
  each object's measured bounds are compared; with `--geometry-mode mesh`
  instead the certified distance between the two surfaces in world
  coordinates, which also sees a reshaping inside unchanged bounds (an
  opening moved along its wall). In that mode both revisions also get
  their exact boundaries, as `check --geometry` builds them, and a body
  with one in both revisions is measured between the boundaries (measure
  `boundary`), so a moved round column is compared too; like every
  measure, a distance straddling the length tolerance is not evaluated. A
  tessellated body without them is not certified, so its geometry is not
  compared in that mode. `--no-exact-boundaries` meshes only.
  `--geometry-mode` or `--no-exact-boundaries` without `--geometry` is a
  usage error (status 2).

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
and the `unit` (`m`, `rad`, or empty for the map scale); a `mesh` or
`boundary` change or undetermined measure also its `witness`: `from`, a
point of the `side`
revision's body that far from the other body, and `to`, its nearest point
found there, in world metres. With `--geometry-mode mesh` the comparison
states `"geometry_mode": "mesh"`. `unresolved` and
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
          missing exact property Pset_WallCommon.Reference
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
    missing exact property Pset_WallCommon.Reference
    id: 5c1f0c9e-6a0b-5d53-9a8e-2f3b8f6c1d20
    decision: accepted by A. Reviewer on 2026-09-27T08:00:00Z: agreed with the architect
```

The summary then counts them (`decisions: 1 accepted · 0 rejected · 0 open ·
3 undecided · 0 changed · 1 stale`), and stale decisions form the
`stale-decision` section, grouped by rule and status and listed with
`--section stale-decisions`. Topics `--decisions-from` could not match form
the `unmatched-topic` section, grouped by reason and topic status, listed with
`--section unmatched-topics` by title and GUID. `--decision accepted|rejected|open|undecided|changed`
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
bridge is not an adapter, because the IFC and Axiolid adapters must not
depend on each other: it is the host layer's `axioval-host-ifc-axiolid`
([host compositions](./architecture.md#host-compositions)), which any other
host links as well. Every object ends in one of four
states:

| State | Meaning | Effect on geometric rules |
|---|---|---|
| exact | every face is planar (polygonal extrusions, including steel sections without fillets or rounded edges, faceted B-reps and polygon meshes whose faces keep within 1 mm of their planes, booleans of these and clips by half-spaces, bounded by a polygon or not), so the mesh is the shape; a whole measured through its parts when every part is exact | measured as exact |
| tessellated | some face is curved, or an authored polygon face is warped more than 1 mm off its plane; the mesh is within the deviation the mesh compiler certifies for it (the 1 mm chord budget, or the bound it computed), and within the reported slab width of a warped face; a whole measured through its parts when some part is tessellated, within the largest deviation of its parts | measured as approximate, never exact |
| no body | the object occupies no material: spatial structure, openings, annotations, grids, ports, structural analysis items, non-products | ignored as an obstacle; an opening is measured as the solid of its void where a rule names it (distance, containment, extents; see [Openings as voids](./clash.md#openings-as-voids)) |
| unmeasured | a physical product that could not be meshed: one with no representation at all and without parts (`no shape representation`, model data; see below), one whose representations include no body and without parts (`no body representation; it has Axis, FootPrint`, naming their identifiers), a whole one of whose parts is unmeasured, or a curved one whose mesh the compiler certifies no deviation for | measurements it could affect are not evaluated; with a bound (below), space measurements it cannot reach are evaluated |

The two bodiless reasons tell model data from engine coverage. A product
whose `Representation` is `$` has no shape at all: its author can fix that,
so besides its unmeasured reason it is reported once per object as the
integrity warning `shape.no-representation` (located
`ifc:<fingerprint>:no-shape:#<id>`), however many rules measure it. It is
never measured as empty: its outcomes stay not evaluated. A product with
representations none of which is a body (an `Axis`, a `FootPrint`, an
identifier the bridge does not read as one) is a gap in the engine or its
adapters, and its reason names the identifiers it has.

A face whose boundary, as written, crosses itself (a quad given in Z order,
a twisted strip quad) or runs back along itself (a spike out to a corner and
back) bounds no region, so the mesh compiler refuses it and the product is
unmeasured with the refusal (`... profile outer ring intersects itself`,
`... folds back on itself at vertex 6`). That too is model data, reported
once per object as the integrity warning `shape.self-intersecting-face`
(located `ifc:<fingerprint>:self-intersecting-face:#<id>`). A hole
joined to the outer boundary by a seam traversed both ways (a keyhole) is
a valid face: since axiolid-construct 0.3.16 (axiolid/kernel#270) it is
triangulated on surface paths, so a face set or surface model carrying one
is measured, while an extruded profile with one is refused by name (`...
runs along a seam both ways (edges <n> and <n>), which a solid cannot
extrude`), not model data. A face whose lowest corner has a `-0.0` twin,
which the clipper found no ear in, is measured since the same release
(axiolid/kernel#269). A keyhole face of a B-rep whose seam ends up as a
triangle edge leaves the solid's mesh not two-manifold
(axiolid/kernel#283): the compiler reports it as a solid whose mesh is
not closed, so it is measured as an open surface, and as an operand of a
boolean it is unmeasured (`... boolean <operand> is a solid whose mesh is
not closed: it encloses no volume to combine`).

A host whose openings remove its whole body, such as a frame modelled as a
wall and voided by the opening meant for the wall around it, compiles to
no triangles. When its gross body, before the openings are subtracted,
compiles to some, the host is unmeasured as `its openings remove its whole
body: nothing is left once the <n> opening(s) voiding it are subtracted`,
never measured as empty, and that is model data, reported once per object
as the integrity warning `shape.voided-body` (located
`ifc:<fingerprint>:voided-body:#<id>`, #310). A body empty before any
opening keeps `mesh compilation produced no triangles`.

Geometry that `ifc-geometry` refuses as impossible as written, such as a
composite curve whose segments leave a gap or a half-space boundary point
off its plane, leaves the product unmeasured as `#<id> (<TYPE>) is
geometrically invalid: <detail>`. That is model data, reported once per
object as the integrity warning `shape.invalid-geometry` (located
`ifc:<fingerprint>:invalid-geometry:#<id>`, #357). An opening that cannot
be subtracted is not: the host's own geometry is valid.

A whole measured through its parts that is unmeasured because a part is
(`no body representation of its own, and its body is the union of its
<n> parts, and part <object> is unmeasured: …`) is model data when every
part is measured, has no body, or is unmeasured for model data: it is
reported once with the code of the part its reason names (located
`ifc:<fingerprint>:<detail>:#<whole>`, such as
`self-intersecting-face`), beside that part's own warning (#357).

A body whose faces, as authored, do not close is an open surface: it has
no inside, so a clash between two such bodies cannot tell touching from
crossing, and the room it takes up above a floor is undecided where it
reaches the floor; a free-floor search bounds it there by the hull of its
vertices instead (#401). Whether faces close is a fact about the file: each
`Body` item that lists its faces (a polygonal or triangulated face set, a
faceted B-rep, a shell- or face-based surface model, through mapped
items) is read item by item, its corners by their exact coordinates, and
an edge that bounds one face only is open as written. A measured product
whose mesh is no closed solid and that has such edges is reported once as
the integrity warning `shape.open-surface` (#311, located
`ifc:<fingerprint>:open-surface:#<id>`), with the number of open edges:
model data, like an `IfcOpenShell` of glazing quads, a tube authored as a
face set stated not closed, or a face set whose faces leave a hole.
A shell whose authored faces close but whose mesh does not is no model
data and gets no warning: the mesh compiler can drop a corner lying a
fraction of a micrometre off its neighbours' line from one face only
(axiolid/kernel#278, #314). Its measurements that need an inside stay not evaluated; the
not-evaluated inventory labels them `(model data)` when every body they
rest on carries the warning. A face set that repeats each corner once per
face is not open: equal coordinates are one corner, and the geometry
store measures it as the closed solid it is (#308).

**Openings are subtracted in their host's frame.** Since `ifc-geometry`
0.13 (openbimrs/ifc#388) a host's body is lowered without its world
placement, each opening by its placement relative to the host's (along the
placement chain the two share), the differences run in that frame, and one
placement with the host's world transform is put above the result. An
opening authored exactly as deep as its wall and flush with its faces
therefore keeps that coincidence under a georeferenced site hundreds of
kilometres out and turned to grid north, where in world coordinates the
rounding of the placements moved the faces apart and the kernel refused the
result as touching itself along the opening's edges (engine#303). An
opening that shares no placement chain with its host is subtracted in world
coordinates as before. The measured shape is the same up to rounding; an
opening that stops a few 1e-15 m short of a face as exported still leaves a
skin that thin, which the volume kernel refuses as self-intersecting once
the body is placed that far out.

The planarity check is conservative. Anything it does not recognise counts as
tessellated, which only loses exactness, never presents an approximation as
exact.

**Openings taken as already applied.** IFC4 Reference View exports author
each `IfcOpeningElement` with only a `Reference` representation (usually
tessellated) against a host whose `Body` already has the hole, and the IFC4
schema states that such a representation "is not subtracted, it is
provided in addition to the hole in the Body shape representation of the
voided element". In an IFC4 or IFC4X3 file, an opening whose every
representation is `Reference` is therefore taken as already applied
(`ifc-geometry` ≥ 0.10, `ReferenceOnlyOpenings::TakeAsApplied`,
openbimrs/ifc#351): its host is measured from its `Body` as authored and
nothing is subtracted for it. The decision is the representation
identifiers' alone, never a guess: an opening with no representation, or
with any other representation beside `Reference`, is refused as before and
leaves its host unmeasured naming it, and an IFC2X3 file states no such
reading, so none is taken there. The result's additive
`geometry.openings_taken_as_applied` lists each host, opening and reason,
the stderr line adds `N opening(s) taken as already applied`, and the
evidence of every measurement of the host names the openings: its locator
carries `;applied-openings:<host>=<opening>+...`. Such an opening's void,
which shelf, plan-area, routing and derived-relationship services read for
bodiless openings, is its `Reference` solid.

**Certified heights of tessellated bodies.** A tessellated body's mesh
lies within its chord deviation of the true surface, so read alone its flat
top may lie that deviation higher: a curved building's slab whose top is a
room's floor would reach into the room. Where the body's construction
states its height exactly, the bridge certifies it from the graph the mesh
is compiled from (`axioval_axiolid::certified_vertical_extent`, #393),
whatever the plan outline: an extrusion whose profile plane lies
horizontal after its placements spans its two caps' elevations exactly,
reached, for any direction with a vertical component; a difference or
clip lies within its subject's (an intersection within either operand's)
without a claim to reach them; a union or several items span the hull of
theirs; a whole measured through its parts spans its parts' vertical
bounds. The elevations are widened by 64 units in the last place of their
magnitude for rounding and registered only where they agree with the mesh
within its deviation (`with_vertical_extent_bounds`); every other body (a
tilted profile plane, a revolution, a swept disk, an authored mesh or
B-rep) keeps the deviation-widened extent. The exact boundary's extent
bounds (`with_extent_bounds`), where there is one, are combined with them:
the tightest of each reading holds. Exact bodies need none.

**Wholes measured through their parts.** A physical product with no `Body`
of its own that is decomposed into parts (`IfcRelAggregates`, read through
the IFC session's relationship edges, at any depth) is measured as the
union of its parts' bodies: a stair of flights and landings, a roof of
slabs, a curtain wall of members and plates, a wall of
`IfcBuildingElementPart` layers, an element assembly. Its mesh is its parts'
meshes side by side; it is exact when every part is, tessellated within the
largest deviation of its parts otherwise, and it has an exact boundary
where every part has one (their items side by side in the world). A part
that occupies no material (an opening, say) adds nothing. The whole fails
closed: if any part is unmeasured, the whole is too, with the reason naming
the first such part in identity order (`no body representation of its own,
and its body is the union of its 2 parts, and part … is unmeasured: …`); a
product with neither a body nor parts stays unmeasured as `no shape
representation` or `no body representation; it has …`. The
result's additive `geometry.composed` counts the wholes measured this way
(they are counted as exact or tessellated too), and the stderr line adds
`N measured through their parts`. Evidence about such a whole states it:
its locator ends in `;union:<whole>=<n>-parts`.

The whole keeps its own identity, separate from its parts', so a rule
selecting both measures both. A whole and its own parts (at any depth),
or two wholes holding a part in common, are one piece of material, so the
pairwise rules (`clash`, `clash-matrix`, `containment`, `distance`) never
pair them: a stair never clashes with its own flight, nor is a part
contained in its own whole. A clash between a crossing wall and a wall of
two layers is reported against the wall and against each layer it crosses,
since the rule selected all three. Volumes are bounded piece by piece, so
parts that overlap are never counted twice: a whole encloses at most the
sum of its parts' volumes and at least that sum less what any two of them
share. Its mesh surface includes the faces where its parts meet, so a body
inside the whole near such a face has its depth measured to it, a lower
bound as every witnessed depth is; a distance from outside is the distance
to the nearest part, which is exactly the distance to the union. Services
that look at every body in a scene (facade areas, guards, space
measurements) see the whole and its parts as separate bodies, as a rule
selecting both asks.

**A whole's openings.** An opening voiding such a whole
(`IfcRelVoidsElement` on the wall, not on its layers) voids its material,
which is its parts'. Where the parts are already cut (a Reference View
export) the opening's body only touches them; where they are uncut (a
design transfer export) it reaches into their material, and the union
alone would fill a door in a layered wall. So each opening of a whole is
compared with each of its measured parts, at any depth, and cuts a part
only where it reaches into the part's material deeper than 10 µm (the
coordinate precision exporters commonly state, within which a hole a part
already carries and the opening that made it are written apart). For two
exact bodies this is decided cell by cell of the opening's mesh: each
connected piece is one convex cell (an extruded rectangle, as most
openings are written), one prism per cap triangle when it is an extrusion
of another profile, and otherwise (a rebated window written as one
faceted Brep) the convex cells the planes of its own faces split its box
into, each wholly inside the piece or wholly outside it by the winding
number of its centre; a piece of more than 64 planes or 4096 cells, or
one that crosses itself, is only bounded by a convex cell holding it,
which can decide it clear but never cut, and that cell may still decide
clear a piece one of whose own cells cannot. The part cuts a cell when
its surface reaches into the cell's core (the cell shrunk by those 10 µm)
or the core lies inside the part (a cavity), by its winding number, and
is clear of it otherwise. Material left in the opening is then at most
20 µm thick, about any face of the opening or between its cells. What
that leaves open, and any tessellated pair, is decided by the certified
volume the two share: one beyond a layer 10 µm thick over the opening's
faces where the two can meet (the area of the opening's faces inside the
part's box) cuts, since material within those 10 µm of the faces fills
no more; one within the rounding of touching bodies (1e-12 m³) does not;
anything between is undecided. A volume never shows that nothing reaches
deeper than 10 µm, so clear stays at rounding. The opening is subtracted from each part it cuts, and from
no other: the part is meshed again, net of its own openings, less the
opening's `Body` lowered by its own placement (in world coordinates), and
the whole is composed from the parts so cut. A part whose cut cannot be
decided (a tessellated part flush with the opening, whose shared volume
lies between that rounding and that layer; a malformed opening piece the
volume cannot be measured against), that lies wholly within the opening, or that the
compiler cannot cut (a net body of several solids is refused by name) is
unmeasured with a reason naming the opening and the whole, and the whole
with it, naming the part. A whole one of whose
openings has no body to compare (`no body representation`, say) is
unmeasured as `no body representation of its own, and whether its opening
… cuts its parts cannot be decided, so it cannot be subtracted from them:
…`, never measured with the opening filled. In an IFC4 or IFC4X3 file an
opening whose every representation is `Reference` is taken as already
applied to the whole's parts, as it is for a host, and listed in
`geometry.openings_taken_as_applied` under the whole. The result's
additive `geometry.whole_openings` lists every opening of a whole decided
by geometry with the parts it was `subtracted_from` (empty where it cuts
none: they are already cut, or it misses them), the stderr line adds `N
opening(s) of wholes subtracted from their parts`, and the evidence of every
measurement of a cut part, and of each whole holding it, names the
openings: its locator carries `;whole-openings:<body>=<opening>+...`. A
cut part is measured cut on its own too: its material does not fill the
opening either.

**Bounds of unmeasured products.** The space service is told where an
unmeasured product's body can be, so that it refuses only the space
measurements the body could reach. A bound may be several boxes: the
body lies within their union, and a measurement is refused only where
one of them reaches. A whole whose parts could not be composed is
bounded by its parts: the boxes of the measured ones and the boxes of the
unmeasured ones, each kept apart (#358), so a space between two parts of
a long railing is not refused by the box spanning both; one part with no
bound leaves the whole without one. Otherwise a product that states a `Box`
representation (`IfcBoundingBox` items) is bounded by that box: its eight
corners are placed as the product's representations are (the context's
world coordinate system above the placement chain) and enclosed. A
product without one whose openings leave it unmeasured is bounded by its
gross body, before the openings are subtracted, when that is measured:
the net body is the gross body less its openings, so it lies inside the
gross mesh's extent grown by the deviation certified for it (#301). Any
other product left unmeasured by its own meshing is bounded by its `Body`
items, each read on its own and its box kept apart (#358): an item that
compiles by its mesh grown by its certified deviation; a boolean difference
or intersection, and a clip by a half-space, by its first operand, since
it only removes material from it (a union by both operands); a polygon
mesh, a triangle mesh or a B-rep with planar faces and straight edges by
the box of its authored vertices, whatever face of it is refused; a linear
extrusion the kernel refuses (one along its profile's plane) by its
profile's box (the kernel's flattened outer ring grown by the deviation it
certifies) swept along the extrusion, one box from end to end; every
placement (instances, mapped items) applied to each box's corners. More
than 256 boxes are joined into the one box around them. An item
whose lowering is refused (a clip whose half-space boundary is invalid)
is read through its IFC boolean or mapped item the same way. Any item
that cannot be bounded so leaves the product without a bound. A whole
measured through its parts is measured and needs no bound. A space
measurement no bound can reach is evaluated; one a bound reaches is not
evaluated and names the product. Without a bound, or with one that cannot
be read, the product may be anywhere and refuses every space measurement
it could change.

The free-space service is also told a closed solid an unmeasured body lies
within, its enclosure (#401), which a free-floor search takes as a possible
obstacle (see [free space](./free-space.md)). A product whose net body is
refused as a difference (a curved wall less openings the exact boolean
cannot cut) is enclosed in the first subject of its differences, outermost
first, that the mesh compiler measures with a certified deviation: its
gross body before its openings, else the subject of an authored difference,
placed by the transform above it (its deviation scaled by the placement's
largest stretch), between the elevations the net body's construction
certifies. A part a whole's opening cannot be subtracted from is enclosed
in its body before that opening. Anything else has no enclosure. Since
axiolid-mesh-compile 0.3.27 a curved wall whose opening or niche is flush
with its curved face (a niche on the inner face, an opening through the
whole thickness, or one a unit in the last place off it) is no longer
refused: the exact boolean reads the coincident faces as one surface within
tolerance (axiolid/kernel#324) and the wall is measured within its
certified deviation (#403). Since axiolid-mesh-compile 0.3.28 a curved wall
whose openings abut each other, one side reaching micrometres to just under
a millimetre past its neighbour's, or reach just past the wall's ends, is
no longer refused as an "exact boolean with a point too close to a face
boundary to classify": the boolean is cut again at a finer tolerance and
such points are classified exactly (axiolid/kernel#332), and the wall is
measured within its certified deviation (#417).

The kernel keeps every surface point of a revolution (tapered too), a sphere,
a torus and a sweep along one circle or ellipse arc within the 1 mm of its
mesh (axiolid/kernel#231). A body that would need more than 4096 steps round
an axis to do so, such as a ring kilometres across, is refused rather than
meshed coarser: it is unmeasured, with the reason `mesh compilation refused:
keeping the surface within the 0.001 m chord tolerance needs more revolution
angular steps than the kernel's budget allows, …`.

Every curved mesh is declared with the deviation the mesh compiler
certifies for it (`compile_mesh_with_deviation`, axiolid/kernel#232): the
1 mm budget where its construction proves it (revolutions, spheres, tori,
cylinders, cones, and disks swept along segments, arcs, composites of them
and filleted polylines), the bound it computed for the mesh at hand
otherwise (curved B-rep faces, disks swept along B-splines and ellipses,
ellipse and spline profiles), larger or smaller than 1 mm, as returned. A
boolean whose result is curved (a wall with a round window, an I-beam cut
by round holes, a roof-clipped wall with a round window) is certified by
measuring its mesh against the kernel's exact result of the same boolean
(axiolid/kernel#235); its bound reads close to the 1 mm asked. Where it
certifies none, the body is unmeasured with the paths it names, never
declared within a tolerance nothing proves: a boolean the exact compiler
refuses (a union; a hole a fraction of the tolerance off a turned beam's
filleted flange: `… where the contact cannot be placed`), a tapered
extrusion, a sectioned spine. An extrusion against its profile normal
(`ExtrudedDirection (0, 0, -1)`, an opening cut down from a slab's top)
is built exactly as the mirror of the forward one since
axiolid-construct 0.3.16 (axiolid/kernel#275), so a boolean with one is
certified like any other. An extrusion oblique to its profile's normal,
of a profile with arcs (a rounded rectangle, a contour with arcs, a
circle), is built exactly since axiolid-construct 0.3.17, each arc's wall
the elliptical cylinder it sweeps (axiolid/kernel#280), so an oblique round
column is measured and has an exact boundary; an oblique ellipse is
refused by the kernel by name (`oblique ellipse extrusion`). The kernel's
exact boolean cuts those elliptical-cylinder walls since
axiolid-mesh-compile 0.3.19 (axiolid/kernel#287), so a slab less an
oblique round shaft or rounded opening, cut up through it or down from
above it, and an oblique rounded member clipped by a half-space are
certified and measured. An extrusion whose direction lies within
the tolerance of its profile's plane sweeps no volume and is refused by
the kernel by name (`extrusion direction in the profile plane`,
axiolid/kernel#281). A round hole touching a planar face (a web
hole touching an I-beam's flange, with or without root fillets) is built
and certified since axiolid-mesh-compile 0.3.13 (axiolid/kernel#243,
#249). A space boundary whose curve-bounded plane is bounded by a
composite or trimmed curve (segments used against their sense included)
is meshed since the same release (axiolid/kernel#255) and measured.

**Warped authored faces.** A face of an `IfcFacetedBrep` or an
`IfcPolygonalFaceSet` whose corners leave its plane by more than the
1 mm tolerance has no single true surface: the two triangulations of a
warped quad, a bilinear patch and the face flattened onto its plane are
all readings of it. The mesh compiler triangulates such a face in its fit
plane (the outer ring's centroid and Newell normal) and reports, for
polygon mesh faces and faceted B-rep faces alike, the width of the slab
its corners (holes included) span about that plane (axiolid/kernel#254,
#257, #261), scaled by each placement's stretch. Every reading lies in
that slab, so the body is declared tessellated within the reported
width: a quad with one corner lifted 5 cm within about 2.5 cm, the gap
between its two triangulations at its centre, a saddle with corners at
`±h` within `2 h`. A warped face never makes a body exact, and a body in
which one is an operand of a boolean (a faceted wall with openings) is
unmeasured, since the compiler bounds no boolean of a polygon mesh or
B-rep and nothing else bounds how far the boolean's result lies from its
mesh. Faces within 1 mm of their plane count as planar, as the compiler
counts them.

**Face triangulation.** Planar faces, faceted B-rep faces and curved
faces' parameter domains are triangulated by the kernel's certified ear
clipper (axiolid/kernel#260), which proves its triangles tile the face
exactly once. Rings that bound no region (a spike folding back, rings
crossing after rounding, overlapping trims), which the earlier
triangulator covered partly without a word, are refused by name, and the
body is unmeasured with the reason. Rings touching at a single point
bound a valid face and are accepted (#262). Where such a point lies
inside an edge another face shares, the compiler welds that face's edge
through it since axiolid-mesh-compile 0.3.15 (axiolid/kernel#265), so the
mesh closes. Every result it labels a solid is checked for closure: one
that does not close is never claimed a solid, so a body whose mesh is
open is measured as an open surface (no volume, no containment inside
it), and is unmeasured as an operand of a boolean, which needs a volume.
A disk swept round a
polyline corner without a fillet radius is mitred at half angle, as
`IfcSweptDiskSolid` defines it, and certified within the budget
(axiolid/kernel#245); with `IfcSweptDiskSolidPolygonal`'s fillet radius
each corner becomes a tangent arc and is certified too. Since
axiolid-mesh-compile 0.3.24 (axiolid/kernel#248) a disk swept along a
closed polyline sweeps as a loop and a corner beside an arc is mitred, both
measured. Still refused by name and unmeasured: a fillet radius equal to
the disk radius (a horn torus) and a mitre reaching past its leg. Planar bodies, booleans of planar operands and clips by
polygonally bounded half-spaces included, stay exact and are meshed
without a deviation report. Since axiolid-mesh-compile 0.3.27 a polygonal
clip is cut through the whole solid it clips, however small its boundary
(axiolid/kernel#325, #412): a tall wall clipped within a small boundary at
its end is measured without the strip an earlier compiler left. Since ifc-geometry 0.28 every `IfcPolygonalBoundedHalfSpace` is one
clip along its `Position`'s Z (openbimrs/ifc#515), so a prism oblique to
its plane (a gable wall under sloped roof planes, #404) is measured for a
non-convex boundary and one with an arc too, where a non-convex or arc
boundary under an oblique prism was refused. The kernel refuses by name,
and leaves unmeasured, a `Position` whose Z lies in the clip's plane and
a clip used as a union's operand, a boolean's first operand or a
body's root. Since axiolid-mesh-compile 0.3.16 the
compiler snaps operand vertices onto the other operand's faces before it
cuts (axiolid/kernel#276), so an opening exported a rounding error short of
its host's face leaves no skin of rounding residue, which self-intersected
once placed at georeferenced coordinates and left the host's volume
unmeasured (32 of 40 such hosts on a 76 MB model, #306). Since 0.3.18 the
snap closes rounding residues only, at most 16 ε times the operands'
largest coordinate magnitude (about 2e-8 m at 6e6 m), and lands only on
exactly axis-aligned faces; a snapped boolean that is refused or pinches
falls back to the unsnapped operands (axiolid/kernel#291, #356). An
authored skin of 1 µm to 1 mm stays geometry. Only the deviation report
states a move (`operands snapped onto each other within tolerance`), and
it measures every boolean against the exact compiler's result, which
tripled that model's meshing time, so a planar boolean is not asked for
it: its snap is undeclared, and at most rounding. Where a report is taken
(a curved body, a planar one with warped faces), a move above 1e-9 m is
declared, and a smaller one is rounding. Measuring a curved boolean costs about
half a second per opening at 1 mm (release build), so a model with many
round openings takes correspondingly longer to mesh. On a 29 MB model
every one of its 56 curved bodies is certified and meshing takes as long
as before; on a small sample house the two upper walls clipped by the
roof are exact.

Exact means the mesh is the solid, and a placement far from the origin
rounds the mesh's coordinates: 5600 km out, as a model placed in map
coordinates puts its bodies, to about 1e-9 m. However exact its
construction, such a mesh is not the solid, and a volume, area or
distance read from it is the rounded mesh's (#368: a planar wall less its
openings measured 2.2075750000614 m3 there, an interval excluding its
true 2.207575). A mesh whose coordinates reach beyond 4096 m is therefore
declared within the rounding of its largest coordinate, 16 machine
epsilons of it (about 1.5e-11 m at the reach, 2e-8 m at 5600 km), and a
tessellated one has that added to its bound, so every service widens what
it reads as it widens a tessellation. Within the reach a body keeps its
exactness: its coordinates round no more than the model's own do, and the
proximity service still widens its own volume by the band of that
rounding.

A model placed far out is therefore measured in a local frame (#372). When
the outermost placement of the first model (a placement relative to
nothing, typically the site's) lies more than 4096 m from the origin in
plan, its plan position becomes the local origin: every placement relative
to nothing in every model has it subtracted from its location, in the
file's units, before any placement is composed, so each chain composes at
the magnitudes it reaches below the site and every mesh is built a few
hundred metres out at most. The chosen location becomes the origin exactly,
and another lying within a factor of two of it moves exactly too; one lying
farther stays far out, where the reach above still counts its rounding.
Heights never move, so storey elevations, vertical extents and heights
above the terrain read alike in both frames. The bodies are then exact
where their construction is, as at the origin. A model is measured in its
model coordinates instead, as before, when any model of the check derives a
placement from curves (an `IfcLinearPlacement`, an `IfcGridPlacement`, an
alignment), states a representation context whose world coordinate system
is rotated, has a product with a body but no placement, or has a placement
whose location cannot be read: every model is measured about one origin, or
none is. `compare` always measures in model coordinates, since it compares
bodies and extents across two sessions.

Everything that is not a length, an area or a volume moves back into model
coordinates where it leaves the engine or meets context stated outside the
model. Object frames and door leaves are answered in the same local frame
as the bodies (`EvidenceSession::with_local_origins`), so clearances,
swings and frame-based positions keep comparing like with like. Context
zones are placed into the local frame (`ContextZone::place_in`, their
accuracy widened by the rounding of the move unless every point moves
exactly), measured plan points are moved back before they are placed on a
terrain, the `x` and `y` measured values, `body_position`, positions named
in findings, reasons and evidence, the zones' placement check and BCF
cameras, section boxes and snapshots are all in model coordinates. The
result's `geometry.local_origin` (additive, absent otherwise) names the
origin in model metres.

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

IFC4X3 geometry families are lowered by `ifc-geometry` (0.10) or refused by
name: alignment curves, gradient curves, open cross profiles and
`IfcTriangulatedIrregularNetwork` terrains are measured. `CUBIC` transitions
and vertical circular arcs and clothoids lower exactly (openbimrs/ifc#90,
#258), and a disk swept along a gradient curve is measured within the
deviation the mesh compiler certifies against the exact tube
(axiolid/kernel#252); a gradient whose grade breaks inside the sweep is
unbounded by name and leaves its object unmeasured. Stations
(`IfcPointByDistanceExpression`, `IfcAxis2PlacementLinear` with its `Axis`
and `RefDirection`), `IfcOffsetCurveByDistances`,
`IfcSectionedSolidHorizontal` and `IfcSectionedSurface` lower exactly onto
stations (openbimrs/ifc#307). Since axiolid-mesh-compile 0.3.24
(axiolid/kernel#247) the mesh compiler certifies the deviation between
stations wherever the frame law along the basis has certified derivative
bounds (lines, polylines, circles, elevated and banked curves with line or
circle plans), refining the stations until the bound fits the chord
tolerance, so such a sectioned solid is measured. Since
axiolid-mesh-compile 0.3.28 it also bounds runs whose frames it reads by
quadrature, against their exact frame law (axiolid/kernel#316): along an
ellipse or a B-spline, and along an offset beside a line, a circle, an
ellipse or a B-spline, constant or by distances. Elsewhere it stays
unmeasured with the reason that the compiler certifies no bound: along
spirals and other intrinsic curves, chains and offsets beside them, skewed
circles and steep tangents, and in a cell beside a mitre on a curved piece
or with a section that changes across a mitre. Since `ifc-geometry` 0.18 (openbimrs/ifc#346) stations also
lower on a plain `IfcCompositeCurve`, an `IfcTrimmedCurve` and a
station-placed composite basis, and one on a joint or tangent
discontinuity reads the incoming segment, as IFC4.3 ADD2 8.9.3.48.3
says; a run of sections or offsets across a joint lowers too (`tests/check.rs`
`with_geometry_stations_on_a_composite_basis_lower`: a deck along a
composite of two polylines, straight or turned at the joint, with a station
on it, reaches the mesh compiler, where it was refused by name). An
`IfcCurveSegment` placed by an `IfcAxis2PlacementLinear` lowers to a
placement at a station (openbimrs/ifc#311). Since `ifc-geometry` 0.20
stations also lower on an offset basis (`IfcOffsetCurve2D`,
`IfcOffsetCurve3D` and `IfcOffsetCurveByDistances` beside lines, polylines
and in-plane circles, openbimrs/ifc#414; `tests/check.rs`
`with_geometry_stations_on_an_offset_basis_lower`, measured since
axiolid-mesh-compile 0.3.28). Since `ifc-geometry`
0.22 a single station on an offset of a gradient, spiral, B-spline or ellipse
basis lowers too, with Axiolid's seam window, so the kernel snaps it onto
its certified joint (openbimrs/ifc#423). Since `ifc-geometry` 0.23 a run of
sections along such a basis lowers with seam windows, mitred at its joints
(openbimrs/ifc#429; `tests/check.rs`
`with_geometry_a_run_along_an_offset_ellipse_lowers`, measured since
axiolid-mesh-compile 0.3.28), and a linear
placement derived on one works through open-ended path pieces
(openbimrs/ifc#427). Since `ifc-geometry` 0.24 an
`IfcOffsetCurveByDistances` along such a basis lowers as a curve, continued
to its basis's ends, which the kernel measures (openbimrs/ifc#433), and a
linear placement on an ellipse or B-spline trimmed at interior parameters
derives through bounded path pieces (openbimrs/ifc#436). Since `ifc-geometry`
0.25 a sectioned solid along such an `IfcOffsetCurveByDistances` lowers
with seam windows (openbimrs/ifc#443), and since axiolid-mesh-compile
0.3.25 the kernel certifies the length of a varying offset along an ellipse
or B-spline (axiolid/kernel#307); the solid then reaches the mesh compiler,
which since 0.3.28 certifies its frames too (axiolid/kernel#316). The
pinned deck there stays unmeasured only for the cell beside the mitre at
the corner its widening offset makes at station 2 (`tests/check.rs`
`with_geometry_a_run_along_offsets_by_distances_of_an_ellipse_reaches_the_mesh_compiler`).
Still refused by name: a station given as `IfcParameterValue`, a station
basis that is an offset of an offset curve (axiolid/kernel#289), a relation mixing
plan-measured and arc-length pieces, and an `IfcSegmentedReferenceCurve`. A product placed by an
`IfcLinearPlacement` is placed where its linear expression puts it: every
lowering derives the frame from the basis curve with the Axiolid reference
curve evaluator (`ifc-geometry` 0.10, `LoweringSession::with_curve_evaluator`
and `product_world_transform_with_evaluator`, openbimrs/ifc#353), in IFC4.3's
(tangent, left, up) frame, so a positive `OffsetLateral` lies to the left of
the curve's direction and the product's Z is up (#355). On a seam between
two segments the frame takes the incoming segment's tangent (`ifc-geometry`
0.19, openbimrs/ifc#409), and on a composite, trimmed or offset basis it
is derived through the evaluator's curve paths (`ifc-geometry` 0.20,
openbimrs/ifc#418). A cached
`CartesianPosition` is checked against the derived position
(`CachedPositionPolicy::Verify`, #354): within the model's precision the
derived frame is used, and farther apart the product is unmeasured with
both positions named, never placed by the cache. An `IfcParameterValue`
along an alignment is refused by name (openbimrs/ifc#347). The derivation
does not yet compose the placement's `PlacementRelTo` or the placement of
the alignment holding the basis curve (openbimrs/ifc#357), so where either
is not the identity, or the curve is held by no representation while an
alignment of the model is placed off the identity, the product is
unmeasured with that reason, never placed where the curve alone would put
it.

With `--geometry` the bridge also registers an alignment service, so the
measured values `station`, `offset`, `height_above_gradient` and
`alignment_*` (see [Alignments](./derived.md#alignments)) are answered
along each `IfcAlignment`. Its centreline is the 3D curve its `Axis`
representation holds (a gradient curve; a banked curve is read by its
gradient line), lowered as a linear placement's basis curve is, else
the gradient curve `ifc-alignment` composes from its horizontal and
vertical layouts. The alignment's placement and its context must be the
identity: one placed off it is refused by name, because placements along
it do not yet compose that placement (openbimrs/ifc#357), so its stations
would not match where its linearly placed products lie. An object's reference point is its placement
origin, a linear placement derived from its basis curve as above, and a
refused placement leaves its values not evaluated with that reason.

There is no point-to-curve projection in the geometry stack, so the foot is
found here and certified. In plan, a foot at distance `d` is a root of
`g(d) = (P - c(d)) . c'(d)`, and `|g'|` is at most `B1^2 + R B2` over a
stretch, where `B1` and `B2` are the certified derivative bounds
`axiolid-evaluate` gives for the elevated curve there and `R` bounds the
point's distance from it. The length is bisected, split first where the
curve may not be `C^2`; a stretch whose midpoint value exceeds that
Lipschitz bound times its half-width holds no foot and is dropped, so what
remains are brackets every foot lies in. A bracket where `g` falls from
positive to negative holds a nearest point; the ends are candidates too.
The nearest candidate wins only when its distance interval lies wholly
below every other's: two that overlap are ambiguous, an undecided one
nearest is too, and an end nearest leaves the point off the range (before
the start or beyond the end, with the distance). Station, offset and height
are evaluated at the winning bracket's ends and widened by the same bounds
over it, with the reference evaluator's accuracy and rounding allowed on
every evaluation, so each is an interval holding the exact value.

Stations are labelled through the station equations the alignment's
referents state (`Pset_Stationing`); without any, the station is the plan
distance. The parameters follow their laws over the foot's stretch: the
plan's curvature from its curvature law (lines, arcs, clothoids and other
polynomial curvature, piecewise and chained), the gradient from the
elevation law (polynomials and circular arcs), and the cant from the
`IfcAlignmentCant` segments, whose transition shapes are all monotone
along a segment; the vertical curvature from the same elevation laws the
profiles read (below). A sinusoidal spiral, a cubic parabola given as a
parametric plan piece and another elevation law (a vertical clothoid) are
refused by name.

Profiles along a station range (`max_along`, `min_along`, `rate_along`) read the same
laws piece by piece, each piece's rate proven from its law: the derivative
of a polynomial curvature or gradient by interval evaluation; a vertical
circular arc's `(1 + grade^2)^(3/2) / R`, bounded by the grades at the
stretch's ends since the grade is monotone along it; and the cant
transitions `D1 + f(xi) (D2 - D1)`, whose slope shapes `f'` (linear, Bloss,
cosine, sine, Helmert) and the Viennese bend's blend all rise to their peak
at mid-segment and fall back, so a stretch's ends and the middle bound
them. Pieces end at the laws' seams and at station equations, and every
`step` where one is stated; a segment's authored cants are kept exact,
values computed in floating point are widened outward. A law with no rate
bound is refused by name, never sampled. The vertical curvature
`z'' / (1 + z'^2)^(3/2)` of a polynomial elevation and its rate
`z''' F - 3 z' z''^2 G` (`F = (1 + z'^2)^(-3/2)`, `G = (1 + z'^2)^(-5/2)`,
both falling in `|z'|`) are bounded by interval arithmetic over the
stretch, so a constant gradient is exactly zero; a vertical circular arc's
is its constant `1 / R`.

The elements of `alignment_elements` are read from the alignment's
`IfcAlignmentHorizontal` and `IfcAlignmentVertical` segments as they state
them, apart from the centreline (so a layout the centreline cannot be
composed from may still list its elements), in nesting order and without
the zero-length segment closing a layout. A horizontal segment's radii are
its `StartRadiusOfCurvature` and `EndRadiusOfCurvature` (`0` a straight
end; `CLOTHOID` is a clothoid, every other transition token a transition);
its stations run from the sum of the lengths before it. A vertical segment
runs from its `StartDistAlong` over its `HorizontalLength`: a constant
gradient is straight, a circular arc states its `RadiusOfCurvature` at
both ends, and a parabola too, but only where its gradients change over
its length as that radius says (`L / (g2 - g1)` within a millionth), else
its ends are unread with both values; a vertical clothoid states neither
end curvature, so its ends are unread. Stations are labelled through the
station equations as positions are.

The same service cuts sections and sweeps clearance envelopes (see
[Sections and clearance envelopes](./derived.md#sections-and-clearance-envelopes))
over the bodies the bridge meshed, along the same centreline: an alignment
placed off the identity is refused by name for them too
(openbimrs/ifc#357). The section at plan distance `d` is the vertical plane
through `C(d)` normal to the plan tangent, `lateral` its horizontal left
normal `L(d)` and `up` `+Z`. A body's mesh is cut only when it is closed
(every directed edge matched by its reverse): every triangle crossing the
plane gives one oriented segment, from a vertex classification shared by
every triangle and a cut point computed once per edge in a canonical order,
so the segments close exactly. The band is every triangle clipped to the
slab within the radius of the plane and projected onto it; the radius is
the body's certified deviation plus the frame's numerical error (the
reference evaluator's accuracy and rounding, scaled by the body's reach and
the curve's derivative bounds).

For a clearance envelope every step `[a, b]` of the range is bounded: a
point `P(s; u, v) = C(s) + u L(s) + v Z` moves at most
`|C'| + |u| |L'|` per metre, and `|L'|` is at most the plan curvature, so
over the step every point lies within `D = h (B1 + |u|max B2 / speed)` of
its place in the section at the step's middle, `h` the half-width and `B1`,
`B2` the certified derivative bounds of `axiolid-evaluate` over the step
(split at continuity breaks). Each triangle of a body is clipped to the
slab within `D` plus the deviation and the frame's error around that
section, projected, and tested against the envelope grown by the same
amount; when no triangle comes that near, the envelope placed there lies
wholly inside or outside the body and its winding number in the mesh says
which. A body near the envelope is decided by sections: a point surely in
both the body's section region and the envelope, at a station of the
range, is a sure intrusion. Otherwise the step is halved, at most ten times
and within 20 000 stretches per body, and a stretch still undecided leaves
the body possible with that stretch named. A range holding more than
100 000 steps is refused.

The same service measures the available sight distance (see
[Sight distances along alignments](./derived.md#sight-distances-along-alignments))
along the same centreline, against the gradient line of its elevation law
and the selected bodies the bridge meshed (through the adapter's
`SightObstacles`). Targets are stepped forward from the eye; a step is
proven clear of the gradient line when the sight line to its middle target
stays above the line by more than the step's movement allows, bounded
piece by piece from the law's certified gradient and halved where the
bound does not suffice (at most 24 times, and never below the evaluation's
error), and clear of the bodies when none comes within
`h (B1 + |u| B2 / speed)` plus the frame's error of that sight line. A
step not proven clear is halved, down to 1 µm; a target is proven blocked
by a sampled point of the gradient line above its sight line or by the
kernel's `line_of_sight`. The last clear and first blocked targets are
brought within 1 mm, within 20 000 steps.
The bridge reads
the bytes with the session's own STEP reader (`read_ifc_step`), so a REAL
written without its decimal point is measured too and reported once, as an
integrity warning.

The result's `geometry` field records the counts and every unmeasured object
with its reason. The summary prints a `geometry:` line and groups unmeasured
objects by reason; `axioval report result.json --section geometry` lists them.
To rank these reasons and the not-evaluated outcomes they cause across many
models, see the [Not-evaluated inventory](./not-evaluated-inventory.md).

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
`axioval:derived.overlapping-group-space`, `axioval:derived.spans-level`,
`axioval:derived.intersects` and `axioval:derived.adjacent-across` (see
[typed host services](./services.md)) as its
`relationship` or as a
`path` step; every other identity still goes to the IFC relationship service.
Every `IfcSpace` is a space, every `IfcDoor`, `IfcWindow` and
`IfcOpeningElement` an opening, and every `IfcWall` and `IfcSlab` (with their
subtypes) a separating element, beside which `adjacent-across` finds the
spaces when the model states no `IfcRelSpaceBoundary` for it. Every
`IfcBuildingStorey` is a level whose
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
